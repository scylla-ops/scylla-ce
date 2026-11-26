//! The `TestApp` helpers of `MailSettingsService`, and a request for a relay with no TLS on a
//! local port.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::platform::v1::mail_settings_service_client::MailSettingsServiceClient;
use scylla_proto::platform::v1::{
    GetMailSettingsRequest, MailSecurity, MailSettings, ResetMailSettingsRequest,
    SendTestMailRequest, SendTestMailResponse, UpdateMailSettingsRequest,
};
use tonic::Status;

use crate::TestApp;

impl TestApp {
    /// Calls `GetMailSettings` as `actor`.
    pub async fn get_mail_settings(&self, actor: ActorId) -> Result<MailSettings, Status> {
        let request = self.authed(actor, GetMailSettingsRequest {}).await;
        let mut client = MailSettingsServiceClient::new(self.channel.clone());
        let response = client.get_mail_settings(request).await?;
        Ok(response.into_inner().settings.expect("settings"))
    }

    /// Calls `UpdateMailSettings` as `actor`.
    pub async fn update_mail_settings(
        &self,
        actor: ActorId,
        request: UpdateMailSettingsRequest,
    ) -> Result<MailSettings, Status> {
        let request = self.authed(actor, request).await;
        let mut client = MailSettingsServiceClient::new(self.channel.clone());
        let response = client.update_mail_settings(request).await?;
        Ok(response.into_inner().settings.expect("settings"))
    }

    /// Calls `ResetMailSettings` as `actor`.
    pub async fn reset_mail_settings(&self, actor: ActorId) -> Result<MailSettings, Status> {
        let request = self.authed(actor, ResetMailSettingsRequest {}).await;
        let mut client = MailSettingsServiceClient::new(self.channel.clone());
        let response = client.reset_mail_settings(request).await?;
        Ok(response.into_inner().settings.expect("settings"))
    }

    /// Calls `SendTestMail` to an address as `actor`.
    pub async fn send_test_mail(
        &self,
        actor: ActorId,
        to: &str,
    ) -> Result<SendTestMailResponse, Status> {
        let message = SendTestMailRequest { to: to.to_owned() };
        let request = self.authed(actor, message).await;
        let mut client = MailSettingsServiceClient::new(self.channel.clone());
        Ok(client.send_test_mail(request).await?.into_inner())
    }
}

/// Makes a request for a relay with no TLS on a port of `127.0.0.1`, with no user name, from
/// `Scylla <no-reply@relay.test>`.
pub fn local_relay(port: u16) -> UpdateMailSettingsRequest {
    UpdateMailSettingsRequest {
        host: "127.0.0.1".to_owned(),
        port: u32::from(port),
        security: MailSecurity::None as i32,
        username: String::new(),
        password: None,
        from_address: "no-reply@relay.test".to_owned(),
        from_name: "Scylla".to_owned(),
    }
}
