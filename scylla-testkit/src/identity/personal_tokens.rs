//! The `TestApp` helpers of the personal tokens, of the settings of the instance, and of the
//! personal token switch of a workspace.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::accounts::v1::personal_token_service_client::PersonalTokenServiceClient;
use scylla_proto::accounts::v1::{
    CreatePersonalTokenRequest, CreatePersonalTokenResponse, ListPersonalTokensRequest,
    ListPersonalTokensResponse, RevokePersonalTokenRequest,
};
use scylla_proto::auth::v1::TokenLimits;
use scylla_proto::platform::v1::settings_service_client::SettingsServiceClient;
use scylla_proto::platform::v1::{
    GetInstanceSettingsRequest, InstanceSettings, UpdateInstanceSettingsRequest,
};
use scylla_proto::workspaces::v1::workspace_service_client::WorkspaceServiceClient;
use scylla_proto::workspaces::v1::UpdateWorkspaceRequest;
use tonic::Status;

use crate::{id, with_token, TestApp};

impl TestApp {
    /// Calls `CreatePersonalToken` with a token, a name and limits, for 90 days.
    pub async fn create_personal_token_with(
        &self,
        token: &str,
        name: &str,
        limits: Option<TokenLimits>,
    ) -> Result<CreatePersonalTokenResponse, Status> {
        let request = CreatePersonalTokenRequest {
            name: name.to_owned(),
            expiry: 0,
            limits,
        };
        let mut client = PersonalTokenServiceClient::new(self.channel.clone());
        let response = client.create_personal_token(with_token(token, request));
        Ok(response.await?.into_inner())
    }

    /// Calls `CreatePersonalToken` as `actor` with a name and limits, for 90 days.
    pub async fn create_personal_token(
        &self,
        actor: ActorId,
        name: &str,
        limits: Option<TokenLimits>,
    ) -> Result<CreatePersonalTokenResponse, Status> {
        let token = self.token(actor).await;
        self.create_personal_token_with(&token, name, limits).await
    }

    /// Calls `ListPersonalTokens` as `actor`.
    pub async fn list_personal_tokens(
        &self,
        actor: ActorId,
    ) -> Result<ListPersonalTokensResponse, Status> {
        let request = self.authed(actor, ListPersonalTokensRequest {}).await;
        let mut client = PersonalTokenServiceClient::new(self.channel.clone());
        Ok(client.list_personal_tokens(request).await?.into_inner())
    }

    /// Calls `RevokePersonalToken` as `actor`.
    pub async fn revoke_personal_token(&self, actor: ActorId, token: &str) -> Result<(), Status> {
        let request = RevokePersonalTokenRequest {
            token_id: Some(id(token)),
        };
        let request = self.authed(actor, request).await;
        let mut client = PersonalTokenServiceClient::new(self.channel.clone());
        client.revoke_personal_token(request).await?;
        Ok(())
    }

    /// Calls `GetInstanceSettings` as `actor`.
    pub async fn get_instance_settings(&self, actor: ActorId) -> Result<InstanceSettings, Status> {
        let request = self.authed(actor, GetInstanceSettingsRequest {}).await;
        let mut client = SettingsServiceClient::new(self.channel.clone());
        let response = client.get_instance_settings(request).await?.into_inner();
        Ok(response.settings.expect("settings"))
    }

    /// Calls `UpdateInstanceSettings` as `actor` with the personal token switch.
    pub async fn set_personal_tokens(
        &self,
        actor: ActorId,
        enabled: bool,
    ) -> Result<InstanceSettings, Status> {
        let request = UpdateInstanceSettingsRequest {
            personal_tokens_enabled: Some(enabled),
        };
        let request = self.authed(actor, request).await;
        let mut client = SettingsServiceClient::new(self.channel.clone());
        let response = client.update_instance_settings(request).await?.into_inner();
        Ok(response.settings.expect("settings"))
    }

    /// Calls `UpdateWorkspace` as `actor` with the personal token switch only.
    pub async fn set_workspace_personal_tokens(
        &self,
        actor: ActorId,
        workspace: WorkspaceId,
        enabled: bool,
    ) -> Result<bool, Status> {
        let request = UpdateWorkspaceRequest {
            workspace_id: Some(id(workspace.to_string())),
            name: None,
            slug: None,
            personal_tokens_enabled: Some(enabled),
        };
        let request = self.authed(actor, request).await;
        let mut client = WorkspaceServiceClient::new(self.channel.clone());
        let response = client.update_workspace(request).await?.into_inner();
        Ok(response
            .workspace
            .expect("workspace")
            .personal_tokens_enabled)
    }
}
