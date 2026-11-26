//! The `TestApp` helpers of the avatars of the users and of the icons of the Apps.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::accounts::v1 as accounts;
use scylla_proto::accounts::v1::profile_service_client::ProfileServiceClient;
use scylla_proto::apps::v1::app_service_client::AppServiceClient;
use scylla_proto::apps::v1::{App, SetAppIconRequest};
use scylla_proto::common::v1::IconChange;
use tonic::Status;

use crate::{id, with_token, TestApp};

impl TestApp {
    /// Calls `UpdateProfile` with an access token and a change of the avatar only. Empty bytes
    /// remove the image, and an empty color gives the default color.
    pub async fn set_avatar(
        &self,
        token: &str,
        image: Option<Vec<u8>>,
        color: Option<&str>,
    ) -> Result<accounts::User, Status> {
        let request = accounts::UpdateProfileRequest {
            display_name: None,
            handle: None,
            avatar_image: image,
            avatar_color: color.map(str::to_owned),
        };
        let mut client = ProfileServiceClient::new(self.channel.clone());
        let response = client.update_profile(with_token(token, request)).await?;
        Ok(response.into_inner().user.expect("user in response"))
    }

    /// Calls `ImportAvatar` with an access token, and returns the user.
    pub async fn import_avatar(
        &self,
        token: &str,
        provider: &str,
        subject: &str,
    ) -> Result<accounts::User, Status> {
        let request = accounts::ImportAvatarRequest {
            provider: provider.to_owned(),
            subject: subject.to_owned(),
        };
        let mut client = ProfileServiceClient::new(self.channel.clone());
        let response = client.import_avatar(with_token(token, request)).await?;
        Ok(response.into_inner().user.expect("user in response"))
    }

    /// Calls `SetAppIcon` as `actor`, and returns the App. `None` sends no icon.
    pub async fn set_app_icon(
        &self,
        actor: ActorId,
        app: &str,
        icon: Option<IconChange>,
    ) -> Result<App, Status> {
        let request = SetAppIconRequest {
            app_id: Some(id(app)),
            icon,
        };
        let request = self.authed(actor, request).await;
        let mut client = AppServiceClient::new(self.channel.clone());
        let response = client.set_app_icon(request).await?.into_inner();
        Ok(response.app.expect("the app"))
    }
}
