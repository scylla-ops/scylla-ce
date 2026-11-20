//! The `TestApp` helpers of the GitHub and GitLab connections, one for each RPC of
//! `VcsConnectionService` (the redirects of the GitHub install are in `installs`), and the
//! answers of the fake provider for the check of a GitLab token.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::integrations::v1::vcs_connection_service_client::VcsConnectionServiceClient;
use scylla_proto::integrations::v1::{
    ConnectGitLabRequest, ConnectGitLabResponse, DeleteVcsConnectionRequest,
    GetDeliveryStatsRequest, GetDeliveryStatsResponse, GetVcsConnectionRequest,
    GetVcsProvidersRequest, ListConnectionRepositoriesRequest, ListConnectionRepositoriesResponse,
    ListVcsConnectionsRequest, ListVcsConnectionsResponse, ReplaceGitLabTokenRequest,
    ReplaceGitLabTokenResponse, StartGitHubInstallRequest, StartGitHubInstallResponse,
    VcsConnection, VcsProviderInfo,
};
use serde_json::{json, Value};
use tonic::transport::Channel;
use tonic::Status;

use super::FakeProvider;
use crate::rpc::page;
use crate::{id, TestApp};

impl TestApp {
    /// A client of `VcsConnectionService`.
    pub(super) fn connections(&self) -> VcsConnectionServiceClient<Channel> {
        VcsConnectionServiceClient::new(self.channel.clone())
    }

    /// Calls `GetVcsProviders` as `actor`.
    pub async fn get_vcs_providers(
        &self,
        actor: ActorId,
        workspace: &str,
    ) -> Result<Vec<VcsProviderInfo>, Status> {
        let request = GetVcsProvidersRequest {
            workspace_id: Some(id(workspace)),
        };
        let request = self.authed(actor, request).await;
        let response = self.connections().get_vcs_providers(request).await?;
        Ok(response.into_inner().providers)
    }

    /// Calls `ListVcsConnections` as `actor`.
    pub async fn list_vcs_connections(
        &self,
        actor: ActorId,
        workspace: &str,
        size: i32,
        token: &str,
    ) -> Result<ListVcsConnectionsResponse, Status> {
        let request = ListVcsConnectionsRequest {
            workspace_id: Some(id(workspace)),
            page: Some(page(size, token)),
        };
        let request = self.authed(actor, request).await;
        let response = self.connections().list_vcs_connections(request).await?;
        Ok(response.into_inner())
    }

    /// Calls `GetVcsConnection` as `actor`.
    pub async fn get_vcs_connection(
        &self,
        actor: ActorId,
        connection: &str,
    ) -> Result<VcsConnection, Status> {
        let request = GetVcsConnectionRequest {
            connection_id: Some(id(connection)),
        };
        let request = self.authed(actor, request).await;
        let response = self.connections().get_vcs_connection(request).await?;
        Ok(response.into_inner().connection.expect("a connection"))
    }

    /// Calls `StartGitHubInstall` as `actor`.
    pub async fn start_github_install(
        &self,
        actor: ActorId,
        workspace: &str,
    ) -> Result<StartGitHubInstallResponse, Status> {
        let request = StartGitHubInstallRequest {
            workspace_id: Some(id(workspace)),
        };
        let request = self.authed(actor, request).await;
        let response = self.connections().start_git_hub_install(request).await?;
        Ok(response.into_inner())
    }

    /// Calls `ConnectGitLab` as `actor`.
    pub async fn connect_gitlab(
        &self,
        actor: ActorId,
        workspace: &str,
        token: &str,
    ) -> Result<ConnectGitLabResponse, Status> {
        let request = ConnectGitLabRequest {
            workspace_id: Some(id(workspace)),
            token: token.to_owned(),
        };
        let request = self.authed(actor, request).await;
        let response = self.connections().connect_git_lab(request).await?;
        Ok(response.into_inner())
    }

    /// Calls `ReplaceGitLabToken` as `actor`.
    pub async fn replace_gitlab_token(
        &self,
        actor: ActorId,
        connection: &str,
        token: &str,
    ) -> Result<ReplaceGitLabTokenResponse, Status> {
        let request = ReplaceGitLabTokenRequest {
            connection_id: Some(id(connection)),
            token: token.to_owned(),
        };
        let request = self.authed(actor, request).await;
        let response = self.connections().replace_git_lab_token(request).await?;
        Ok(response.into_inner())
    }

    /// Calls `DeleteVcsConnection` as `actor`.
    pub async fn delete_vcs_connection(
        &self,
        actor: ActorId,
        connection: &str,
    ) -> Result<(), Status> {
        let request = DeleteVcsConnectionRequest {
            connection_id: Some(id(connection)),
        };
        let request = self.authed(actor, request).await;
        self.connections().delete_vcs_connection(request).await?;
        Ok(())
    }

    /// Calls `ListConnectionRepositories` as `actor`, for the link of a project.
    pub async fn list_connection_repositories(
        &self,
        actor: ActorId,
        project: &str,
        connection: &str,
        search: &str,
        size: i32,
        token: &str,
    ) -> Result<ListConnectionRepositoriesResponse, Status> {
        let request = ListConnectionRepositoriesRequest {
            project_id: Some(id(project)),
            connection_id: Some(id(connection)),
            search: search.to_owned(),
            page: Some(page(size, token)),
            workspace_id: None,
        };
        self.send_list_connection_repositories(actor, request).await
    }

    /// Sends a request of `ListConnectionRepositories` as `actor`.
    pub async fn send_list_connection_repositories(
        &self,
        actor: ActorId,
        request: ListConnectionRepositoriesRequest,
    ) -> Result<ListConnectionRepositoriesResponse, Status> {
        let request = self.authed(actor, request).await;
        let response = self
            .connections()
            .list_connection_repositories(request)
            .await?;
        Ok(response.into_inner())
    }

    /// Calls `GetDeliveryStats` as `actor`.
    pub async fn get_delivery_stats(
        &self,
        actor: ActorId,
        request: GetDeliveryStatsRequest,
    ) -> Result<GetDeliveryStatsResponse, Status> {
        let request = self.authed(actor, request).await;
        let response = self.connections().get_delivery_stats(request).await?;
        Ok(response.into_inner())
    }

    /// Gives the link of a project the GitLab webhook `hook`, with a sealed secret.
    pub async fn set_link_hook(&self, project: &str, hook: &str) {
        sqlx::query(
            "UPDATE repository_links SET hook_id = $2, sealed_hook_secret = '\\x00'
             WHERE project_id = $1::uuid",
        )
        .bind(project)
        .bind(hook)
        .execute(&self.pool)
        .await
        .expect("set the hook of a link");
    }

    /// Returns the sealed token of a connection, or `None` for a connection with no token.
    pub async fn sealed_token_of(&self, connection: &str) -> Option<Vec<u8>> {
        sqlx::query_scalar("SELECT sealed_token FROM vcs_connections WHERE id = $1::uuid")
            .bind(connection)
            .fetch_one(&self.pool)
            .await
            .expect("read a connection")
    }
}

impl FakeProvider {
    /// Answers the check of a GitLab token of the user `user`: the scopes, the expiry date or
    /// `null`, the bot flag, and the projects with their access level.
    pub fn answer_gitlab_token(
        &self,
        user: (u64, &str, bool),
        scopes: &[&str],
        expires_at: Value,
        projects: &[(u64, &str, u32)],
    ) {
        let token = json!({ "scopes": scopes, "expires_at": expires_at, "active": true });
        self.respond(
            "GET",
            "/api/v4/personal_access_tokens/self",
            200,
            token,
            &[],
        );
        let (id, username, bot) = user;
        let user = json!({ "id": id, "username": username, "avatar_url": null, "bot": bot });
        self.respond("GET", "/api/v4/user", 200, user, &[]);
        let items: Vec<Value> = projects
            .iter()
            .map(|(id, path, level)| {
                json!({
                    "id": id,
                    "path_with_namespace": path,
                    "web_url": format!("https://gitlab.com/{path}"),
                    "default_branch": "main",
                    "visibility": "private",
                    "permissions": {
                        "project_access": { "access_level": level },
                        "group_access": null,
                    },
                })
            })
            .collect();
        self.respond("GET", "/api/v4/projects", 200, Value::Array(items), &[]);
    }
}
