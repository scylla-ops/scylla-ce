//! The `TestApp` helpers of `UserStateService`: the state of the web app for the caller.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::accounts::v1 as pb;
use scylla_proto::accounts::v1::user_state_service_client::UserStateServiceClient;
use tonic::transport::Channel;
use tonic::Status;

use crate::TestApp;

impl TestApp {
    /// Returns a client of `UserStateService`.
    pub fn user_state(&self) -> UserStateServiceClient<Channel> {
        UserStateServiceClient::new(self.channel.clone())
    }

    /// Calls `GetUserState` as `actor`.
    pub async fn get_user_state(&self, actor: ActorId) -> Result<Vec<pb::UserStateEntry>, Status> {
        let request = self.authed(actor, pb::GetUserStateRequest {}).await;
        let response = self.user_state().get_user_state(request).await?;
        Ok(response.into_inner().entries)
    }

    /// Calls `PutUserStateEntry` as `actor`.
    pub async fn put_user_state(
        &self,
        actor: ActorId,
        key: &str,
        value_json: &str,
        expected_version: Option<i64>,
    ) -> Result<pb::UserStateEntry, Status> {
        let request = pb::PutUserStateEntryRequest {
            key: key.to_owned(),
            value_json: value_json.to_owned(),
            expected_version,
        };
        let request = self.authed(actor, request).await;
        let response = self.user_state().put_user_state_entry(request).await?;
        Ok(response.into_inner().entry.expect("entry in response"))
    }
}
