//! The gRPC handler of `TokenService`. Its RPCs need no token: they give one.

use scylla_domain::identity::actors::Actor;
use scylla_domain::identity::users::KIND;
use scylla_extension::AccessError;
use scylla_proto::auth::v1 as pb;
use scylla_proto::auth::v1::token_service_server::{TokenService, TokenServiceServer};
use tonic::metadata::{MetadataMap, MetadataValue};
use tonic::{Request, Response, Status};

use crate::features::identity::accounts::{deliver, to_proto, user_agent, AccountContext};
use crate::features::identity::agents;
use crate::features::identity::providers::Providers;
use crate::features::identity::sessions::{self, cookie};
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
    async fn exchange_agent_assertion(
        &self,
        request: Request<pb::ExchangeAgentAssertionRequest>,
    ) -> Result<Response<pb::ExchangeAgentAssertionResponse>, Status> {
        let request = request.into_inner();
        let ctx = self.ctx();
        let agent =
            agents::verify_assertion(&ctx.pool, &request.assertion, &request.signature).await?;
        let token = ctx.gate.issue(&agent, None).map_err(Error::from)?;
        tracing::info!(actor = %agent.id, kind = agent.kind.as_str(), "token issued");
        Ok(Response::new(pb::ExchangeAgentAssertionResponse {
            token: Some(to_proto(token)),
        }))
    }

    async fn exchange_id_token(
        &self,
        request: Request<pb::ExchangeIdTokenRequest>,
    ) -> Result<Response<pb::ExchangeIdTokenResponse>, Status> {
        let agent = user_agent(request.metadata());
        let id_token = request.into_inner().id_token;
        let signed_in = self
            .providers
            .sign_in_with_id_token(&id_token, agent.as_deref());
        let (user, opened) = signed_in.await?;
        let token = self
            .ctx()
            .gate
            .issue(&user, Some(opened.session))
            .map_err(Error::from)?;
        tracing::info!(actor = %user.id, kind = user.kind.as_str(), "token issued");
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
            .issue(&actor, Some(opened.session))
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
    if !origin.is_some_and(|origin| {
        ctx.settings
            .web_origins
            .iter()
            .any(|allowed| allowed == origin)
    }) {
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
