//! The gRPC handler of `MailSettingsService`: the instance admins read and change the relay of
//! the instance, and send a test mail through it.

use scylla_domain::authz::Permission;
use scylla_domain::identity::users::Email;
use scylla_domain::platform::mail::MailSettingsHolder;
use scylla_domain::platform::Platform;
use scylla_proto::platform::v1 as pb;
use scylla_proto::platform::v1::mail_settings_service_server::{
    MailSettingsService, MailSettingsServiceServer,
};
use tonic::{Request, Response, Status};

use super::{convert, service};
use crate::api::parse::ToProto;
use crate::features::identity::accounts::AccountContext;
use crate::Error;

/// Makes the service.
pub(crate) fn server(ctx: AccountContext) -> MailSettingsServiceServer<MailSettingsGrpc> {
    MailSettingsServiceServer::new(MailSettingsGrpc { ctx })
}

/// The handler.
pub(crate) struct MailSettingsGrpc {
    /// The pool, the gate, the key of the secrets and the mailer of the instance.
    ctx: AccountContext,
}

#[tonic::async_trait]
impl MailSettingsService for MailSettingsGrpc {
    async fn get_mail_settings(
        &self,
        request: Request<pb::GetMailSettingsRequest>,
    ) -> Result<Response<pb::GetMailSettingsResponse>, Status> {
        self.admin(&request, MailSettingsHolder::GET_MAIL_SETTINGS)
            .await?;
        let settings = service::get(&self.ctx).await?;
        Ok(Response::new(pb::GetMailSettingsResponse {
            settings: Some(settings.to_proto()),
        }))
    }

    async fn update_mail_settings(
        &self,
        request: Request<pb::UpdateMailSettingsRequest>,
    ) -> Result<Response<pb::UpdateMailSettingsResponse>, Status> {
        self.admin(&request, MailSettingsHolder::UPDATE_MAIL_SETTINGS)
            .await?;
        let (relay, password) = convert::change(request.into_inner())?;
        let settings = service::update(&self.ctx, relay, password).await?;
        Ok(Response::new(pb::UpdateMailSettingsResponse {
            settings: Some(settings.to_proto()),
        }))
    }

    async fn reset_mail_settings(
        &self,
        request: Request<pb::ResetMailSettingsRequest>,
    ) -> Result<Response<pb::ResetMailSettingsResponse>, Status> {
        self.admin(&request, MailSettingsHolder::RESET_MAIL_SETTINGS)
            .await?;
        let settings = service::reset(&self.ctx).await?;
        Ok(Response::new(pb::ResetMailSettingsResponse {
            settings: Some(settings.to_proto()),
        }))
    }

    async fn send_test_mail(
        &self,
        request: Request<pb::SendTestMailRequest>,
    ) -> Result<Response<pb::SendTestMailResponse>, Status> {
        self.admin(&request, MailSettingsHolder::SEND_TEST_MAIL)
            .await?;
        let to = Email::parse(request.into_inner().to)
            .map_err(|error| Error::invalid("to", error.to_string()))?;
        let outcome = service::test(&self.ctx, &to).await?;
        Ok(Response::new(outcome.to_proto()))
    }
}

impl MailSettingsGrpc {
    /// Authenticates the caller, and checks the route permission on the platform.
    async fn admin<T: Sync>(
        &self,
        request: &Request<T>,
        permission: Permission<Platform>,
    ) -> Result<(), Error> {
        let caller = self.ctx.gate.caller(request).await?;
        self.ctx
            .gate
            .authorize(&caller, permission.on(Platform::ID))
            .await?;
        Ok(())
    }
}
