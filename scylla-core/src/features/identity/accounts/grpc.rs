//! The gRPC handler of `AccountService`. Its RPCs need no token: they make accounts and
//! sessions.

use scylla_extension::{Binding, IssuedToken};
use scylla_proto::accounts::v1 as pb;
use scylla_proto::accounts::v1::account_service_server::{AccountService, AccountServiceServer};
use scylla_proto::auth::v1::IssuedToken as PbToken;
use tonic::metadata::MetadataValue;
use tonic::{Request, Response, Status};

use super::reset;
use super::service::{self, SignUp};
use super::AccountContext;
use crate::api::parse;
use crate::features::identity::providers;
use crate::features::identity::sessions::{cookie, Client, Opened};
use crate::Error;

/// Makes the service.
pub(crate) fn server(ctx: AccountContext) -> AccountServiceServer<AccountsGrpc> {
    AccountServiceServer::new(AccountsGrpc { ctx })
}

/// The handler.
pub(crate) struct AccountsGrpc {
    /// What the account routes share.
    ctx: AccountContext,
}

#[tonic::async_trait]
impl AccountService for AccountsGrpc {
    async fn sign_up(
        &self,
        request: Request<pb::SignUpRequest>,
    ) -> Result<Response<pb::SignUpResponse>, Status> {
        let request = request.into_inner();
        let input = SignUp {
            email: parse::field(request.email, "email")?,
            password: parse::field(request.password, "password")?,
            display_name: parse::field(request.display_name, "display_name")?,
            invitation: Some(request.invitation_token).filter(|token| !token.is_empty()),
        };
        service::sign_up(&self.ctx, input).await?;
        Ok(Response::new(pb::SignUpResponse {}))
    }

    async fn verify_email(
        &self,
        request: Request<pb::VerifyEmailRequest>,
    ) -> Result<Response<pb::VerifyEmailResponse>, Status> {
        service::verify_email(&self.ctx, &request.into_inner().token).await?;
        Ok(Response::new(pb::VerifyEmailResponse {}))
    }

    async fn sign_in(
        &self,
        request: Request<pb::SignInRequest>,
    ) -> Result<Response<pb::SignInResponse>, Status> {
        // Check the identifier and the password, and open a session.
        let client = Client::of_grpc(&request, self.ctx.settings.trust_forwarded);
        let request = request.into_inner();
        let (actor, opened) =
            service::sign_in(&self.ctx, request.identifier, request.password, &client).await?;

        // Sign the first token, and give the refresh token in the response or in a cookie.
        let token = self
            .ctx
            .gate
            .issue(&actor, Binding::Session(opened.session))
            .map_err(Error::from)?;
        let (refresh_token, set_cookie) = deliver(&self.ctx, opened, request.cookie);
        let mut response = Response::new(pb::SignInResponse {
            token: Some(to_proto(token)),
            refresh_token,
        });
        if let Some(value) = set_cookie {
            response.metadata_mut().insert("set-cookie", value);
        }
        Ok(response)
    }

    async fn request_password_reset(
        &self,
        request: Request<pb::RequestPasswordResetRequest>,
    ) -> Result<Response<pb::RequestPasswordResetResponse>, Status> {
        let email = parse::field(request.into_inner().email, "email")?;
        reset::request_reset(&self.ctx, email);
        Ok(Response::new(pb::RequestPasswordResetResponse {}))
    }

    async fn reset_password(
        &self,
        request: Request<pb::ResetPasswordRequest>,
    ) -> Result<Response<pb::ResetPasswordResponse>, Status> {
        let request = request.into_inner();
        let password = parse::field(request.password, "password")?;
        reset::reset_password(&self.ctx, &request.token, password).await?;
        Ok(Response::new(pb::ResetPasswordResponse {}))
    }

    async fn list_sign_in_methods(
        &self,
        _request: Request<pb::ListSignInMethodsRequest>,
    ) -> Result<Response<pb::ListSignInMethodsResponse>, Status> {
        let providers = providers::list_enabled(&self.ctx.pool).await?;
        Ok(Response::new(pb::ListSignInMethodsResponse {
            providers: providers
                .into_iter()
                .map(|provider| pb::SignInProvider {
                    slug: provider.slug,
                    display_name: provider.display_name,
                    kind: provider.kind,
                })
                .collect(),
            sign_up_mode: self.ctx.settings.sign_up_mode.as_str().to_owned(),
        }))
    }
}

/// Gives a refresh token to a client: in the response, or in the session cookie of a browser.
/// Returns the text for the response, and the `set-cookie` value.
pub(crate) fn deliver(
    ctx: &AccountContext,
    opened: Opened,
    cookie: bool,
) -> (String, Option<MetadataValue<tonic::metadata::Ascii>>) {
    if !cookie {
        return (opened.refresh, None);
    }
    let secure = ctx.settings.secure_cookies();
    let value = cookie::set(
        cookie::SESSION,
        &opened.refresh,
        ctx.settings.session_ttl,
        secure,
    );
    (String::new(), MetadataValue::try_from(value).ok())
}

/// Makes the proto of a token.
pub(crate) fn to_proto(token: IssuedToken) -> PbToken {
    PbToken {
        value: token.value,
        expires_at: Some(parse::timestamp(token.expires_at)),
    }
}
