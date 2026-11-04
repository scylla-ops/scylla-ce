//! The gRPC handler of `TokenService`. Its RPCs need no token: they give one.

use scylla_domain::identity::actors::Actor;
use scylla_domain::identity::api_tokens::PERSONAL_TOKEN_PREFIX;
use scylla_domain::identity::apps::AppId;
use scylla_domain::identity::users::KIND;
use scylla_extension::{AccessError, Binding};
use scylla_proto::auth::v1 as pb;
use scylla_proto::auth::v1::token_service_server::{TokenService, TokenServiceServer};
use tonic::metadata::{MetadataMap, MetadataValue};
use tonic::{Request, Response, Status};

use crate::access::attenuate;
use crate::api::parse::{self, ToProto};
use crate::features::identity::accounts::{deliver, to_proto, AccountContext};
use crate::features::identity::providers::Providers;
use crate::features::identity::sessions::{self, cookie, Client};
use crate::features::identity::{apps, personal_tokens};
use crate::Error;

/// Makes the service.
pub(crate) fn server(providers: Providers) -> TokenServiceServer<TokensGrpc> {
    TokenServiceServer::new(TokensGrpc { providers })
}

/// The handler.
pub(crate) struct TokensGrpc {
    /// The identity providers, with what the account routes share.
    providers: Providers,
}

#[tonic::async_trait]
impl TokenService for TokensGrpc {
    async fn exchange_app_assertion(
        &self,
        request: Request<pb::ExchangeAppAssertionRequest>,
    ) -> Result<Response<pb::ExchangeAppAssertionResponse>, Status> {
        let request = request.into_inner();
        let ctx = self.ctx();
        let (app, key) =
            apps::verify_assertion(&ctx.pool, &request.assertion, &request.signature).await?;
        let token = ctx
            .gate
            .issue(&app, Binding::Key(key))
            .map_err(Error::from)?;
        tracing::info!(event = "app.signed_in", app = %app.id, %key);
        Ok(Response::new(pb::ExchangeAppAssertionResponse {
            token: Some(to_proto(token)),
        }))
    }

    async fn exchange_api_token(
        &self,
        request: Request<pb::ExchangeApiTokenRequest>,
    ) -> Result<Response<pb::ExchangeApiTokenResponse>, Status> {
        let ctx = self.ctx();
        let secret = request.into_inner().api_token;
        let exchanged = match secret.starts_with(PERSONAL_TOKEN_PREFIX) {
            true => personal_tokens::exchange(&ctx.pool, &secret).await?,
            false => apps::exchange_token(&ctx.pool, &secret).await?,
        };
        let (actor, token, limits) = exchanged;
        let gate = &ctx.gate;
        let mut issued = gate
            .issue(&actor, Binding::ApiToken(token))
            .map_err(Error::from)?;
        issued.value = attenuate(&issued.value, &limits).map_err(Error::from)?;
        tracing::info!(event = "api_token.exchanged", actor = %actor.id, %token);
        Ok(Response::new(pb::ExchangeApiTokenResponse {
            token: Some(to_proto(issued)),
        }))
    }

    async fn exchange_ci_token(
        &self,
        request: Request<pb::ExchangeCiTokenRequest>,
    ) -> Result<Response<pb::ExchangeCiTokenResponse>, Status> {
        let ctx = self.ctx();
        let request = request.into_inner();
        let app: AppId = parse::required(request.app_id, "app_id")?;
        let audience = &ctx.settings.api_url;
        let id_token = &request.id_token;
        let exchanged = apps::exchange_ci(&ctx.pool, &self.providers, audience, app, id_token);
        let (actor, limits) = exchanged.await?;
        let mut issued = ctx.gate.issue(&actor, Binding::None).map_err(Error::from)?;
        issued.value = attenuate(&issued.value, &limits).map_err(Error::from)?;
        Ok(Response::new(pb::ExchangeCiTokenResponse {
            token: Some(to_proto(issued)),
        }))
    }

    async fn join_app(
        &self,
        request: Request<pb::JoinAppRequest>,
    ) -> Result<Response<pb::JoinAppResponse>, Status> {
        let request = request.into_inner();
        let label = apps::optional_label(request.label)?;
        let pool = &self.ctx().pool;
        let joined = apps::join(
            pool,
            &request.join_code,
            &request.public_key,
            label.as_ref(),
        );
        let (app, key) = joined.await?;
        Ok(Response::new(pb::JoinAppResponse {
            app_id: Some(app.to_proto()),
            key_id: Some(key.to_proto()),
        }))
    }

    async fn exchange_id_token(
        &self,
        request: Request<pb::ExchangeIdTokenRequest>,
    ) -> Result<Response<pb::ExchangeIdTokenResponse>, Status> {
        let client = Client::of_grpc(&request, self.ctx().settings.trust_forwarded);
        let id_token = request.into_inner().id_token;
        let signed_in = self.providers.sign_in_with_id_token(&id_token, &client);
        let (user, opened) = signed_in.await?;
        let token = self
            .ctx()
            .gate
            .issue(&user, Binding::Session(opened.session))
            .map_err(Error::from)?;
        Ok(Response::new(pb::ExchangeIdTokenResponse {
            token: Some(to_proto(token)),
            refresh_token: opened.refresh,
        }))
    }

    async fn refresh(
        &self,
        request: Request<pb::RefreshRequest>,
    ) -> Result<Response<pb::RefreshResponse>, Status> {
        // Read the refresh token, then rotate it.
        let ctx = self.ctx();
        let (refresh, from_cookie) = refresh_token(ctx, request.metadata(), request.get_ref())?;
        let (user, opened) = sessions::refresh(&ctx.pool, &refresh).await?;

        // Sign a token in the session, and give the new refresh token back the same way.
        let actor = Actor {
            id: user,
            kind: KIND,
        };
        let token = ctx
            .gate
            .issue(&actor, Binding::Session(opened.session))
            .map_err(Error::from)?;
        let (refresh_token, set_cookie) = deliver(ctx, opened, from_cookie);
        let mut response = Response::new(pb::RefreshResponse {
            token: Some(to_proto(token)),
            refresh_token,
        });
        if let Some(value) = set_cookie {
            response.metadata_mut().insert("set-cookie", value);
        }
        Ok(response)
    }

    async fn sign_out(
        &self,
        request: Request<pb::SignOutRequest>,
    ) -> Result<Response<pb::SignOutResponse>, Status> {
        let ctx = self.ctx();
        let request_body = pb::RefreshRequest {
            refresh_token: request.get_ref().refresh_token.clone(),
        };
        let (refresh, from_cookie) = refresh_token(ctx, request.metadata(), &request_body)?;
        sessions::close(&ctx.pool, &refresh).await?;
        let mut response = Response::new(pb::SignOutResponse {});
        let clear = cookie::clear(cookie::SESSION, ctx.settings.secure_cookies());
        if let (true, Ok(value)) = (from_cookie, MetadataValue::try_from(clear)) {
            response.metadata_mut().insert("set-cookie", value);
        }
        Ok(response)
    }
}

impl TokensGrpc {
    /// Returns what the account routes share.
    fn ctx(&self) -> &AccountContext {
        &self.providers.ctx
    }
}

/// Reads the refresh token of a request: in its body, or in the session cookie of a browser.
/// A cookie needs an allowed web origin, so another site cannot use it. Returns the token, and
/// `true` if it came from the cookie.
fn refresh_token(
    ctx: &AccountContext,
    metadata: &MetadataMap,
    request: &pb::RefreshRequest,
) -> Result<(String, bool), Error> {
    if !request.refresh_token.is_empty() {
        return Ok((request.refresh_token.clone(), false));
    }
    let origin = metadata.get("origin").and_then(|value| value.to_str().ok());
    if !origin.is_some_and(|origin| ctx.settings.is_web_origin(origin)) {
        let reason = "a cookie refresh needs an allowed web origin".to_owned();
        return Err(AccessError::Denied(reason).into());
    }
    let headers = metadata
        .get_all("cookie")
        .iter()
        .filter_map(|value| value.to_str().ok());
    let refresh = cookie::read(headers, cookie::SESSION)
        .ok_or_else(|| AccessError::Unauthenticated("no session cookie".to_owned()))?;
    Ok((refresh, true))
}
