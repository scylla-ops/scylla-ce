//! The `TestApp` helpers of the repository links, one for each RPC of `RepositoryService`, the
//! webhook of a link in the database, and the answers of the fake provider for a GitLab project,
//! its webhook, and the branches and the tags of a repository.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::integrations::v1::repository_service_client::RepositoryServiceClient;
use scylla_proto::integrations::v1::{
    GetRepositoryLinkRequest, GitRefKind, LinkRepositoryRequest, ListRefsRequest, ListRefsResponse,
    RepositoryLink, TestDeliveryRequest, TestDeliveryResponse, UnlinkRepositoryRequest,
    UpdateRepositoryLinkRequest,
};
use serde_json::{json, Value};
use tonic::transport::Channel;
use tonic::Status;

use super::FakeProvider;
use crate::rpc::page;
use crate::{id, TestApp};

impl TestApp {
    /// A client of `RepositoryService`.
    fn repositories(&self) -> RepositoryServiceClient<Channel> {
        RepositoryServiceClient::new(self.channel.clone())
    }

    /// Calls `GetRepositoryLink` as `actor`. A project with no link gives `None`.
    pub async fn get_repository_link(
        &self,
        actor: ActorId,
        project: &str,
    ) -> Result<Option<RepositoryLink>, Status> {
        let request = GetRepositoryLinkRequest {
            project_id: Some(id(project)),
        };
        let request = self.authed(actor, request).await;
        let response = self.repositories().get_repository_link(request).await?;
        Ok(response.into_inner().link)
    }

    /// Calls `LinkRepository` as `actor`.
    pub async fn link_repository(
        &self,
        actor: ActorId,
        project: &str,
        connection: &str,
        external_id: &str,
    ) -> Result<RepositoryLink, Status> {
        let request = LinkRepositoryRequest {
            project_id: Some(id(project)),
            connection_id: Some(id(connection)),
            external_id: external_id.to_owned(),
        };
        let request = self.authed(actor, request).await;
        let response = self.repositories().link_repository(request).await?;
        Ok(response.into_inner().link.expect("a link"))
    }

    /// Calls `UpdateRepositoryLink` as `actor`, with the changes of `changes` on `project`.
    pub async fn update_repository_link(
        &self,
        actor: ActorId,
        project: &str,
        changes: UpdateRepositoryLinkRequest,
    ) -> Result<RepositoryLink, Status> {
        let request = UpdateRepositoryLinkRequest {
            project_id: Some(id(project)),
            ..changes
        };
        let request = self.authed(actor, request).await;
        let response = self.repositories().update_repository_link(request).await?;
        Ok(response.into_inner().link.expect("a link"))
    }

    /// Calls `UnlinkRepository` as `actor`.
    pub async fn unlink_repository(&self, actor: ActorId, project: &str) -> Result<(), Status> {
        let request = UnlinkRepositoryRequest {
            project_id: Some(id(project)),
        };
        let request = self.authed(actor, request).await;
        self.repositories().unlink_repository(request).await?;
        Ok(())
    }

    /// Calls `TestDelivery` as `actor`.
    pub async fn test_delivery(
        &self,
        actor: ActorId,
        project: &str,
    ) -> Result<TestDeliveryResponse, Status> {
        let request = TestDeliveryRequest {
            project_id: Some(id(project)),
        };
        let request = self.authed(actor, request).await;
        let response = self.repositories().test_delivery(request).await?;
        Ok(response.into_inner())
    }

    /// Calls `ListRefs` as `actor`.
    pub async fn list_refs(
        &self,
        actor: ActorId,
        project: &str,
        kind: GitRefKind,
        search: &str,
        size: i32,
        token: &str,
    ) -> Result<ListRefsResponse, Status> {
        let request = ListRefsRequest {
            project_id: Some(id(project)),
            kind: kind as i32,
            search: search.to_owned(),
            page: Some(page(size, token)),
        };
        let request = self.authed(actor, request).await;
        let response = self.repositories().list_refs(request).await?;
        Ok(response.into_inner())
    }

    /// Returns the GitLab webhook of the link of a project and its sealed signing token, or
    /// `None` for a link with no webhook.
    pub async fn hook_of(&self, project: &str) -> Option<(String, Vec<u8>)> {
        let row: (Option<String>, Option<Vec<u8>>) = sqlx::query_as(
            "SELECT hook_id, sealed_hook_secret FROM repository_links WHERE project_id = $1::uuid",
        )
        .bind(project)
        .fetch_one(&self.pool)
        .await
        .expect("read a link");
        row.0.zip(row.1)
    }
}

impl FakeProvider {
    /// Answers `GET /projects/:id` of GitLab: the project `path`, where the service account has
    /// the access `level` (30 is Developer, 40 Maintainer).
    pub fn answer_gitlab_project(&self, project: u64, path: &str, level: u32) {
        let found = json!({
            "id": project,
            "path_with_namespace": path,
            "web_url": format!("https://gitlab.com/{path}"),
            "default_branch": "main",
            "visibility": "private",
            "permissions": {
                "project_access": { "access_level": level },
                "group_access": null,
            },
        });
        let route = format!("/api/v4/projects/{project}");
        self.respond("GET", &route, 200, found, &[]);
    }

    /// Answers the webhook that Scylla makes on a GitLab project with the id `hook`, and its
    /// delete.
    pub fn answer_gitlab_hook(&self, project: u64, hook: u64) {
        let route = format!("/api/v4/projects/{project}/hooks");
        self.respond("POST", &route, 201, json!({ "id": hook }), &[]);
        let route = format!("{route}/{hook}");
        self.respond("DELETE", &route, 204, Value::Null, &[]);
    }

    /// Answers the branches and the tags of the GitHub repository `full_name` (`(name, sha)`),
    /// with the installation token `ghs_refs` of the installation.
    pub fn answer_github_refs(
        &self,
        installation: u64,
        full_name: &str,
        branches: &[(&str, &str)],
        tags: &[(&str, &str)],
    ) {
        let token = json!({ "token": "ghs_refs", "expires_at": "2099-01-01T00:00:00Z" });
        let route = format!("/app/installations/{installation}/access_tokens");
        self.respond("POST", &route, 201, token, &[]);
        for (kind, refs) in [("branches", branches), ("tags", tags)] {
            let items: Vec<Value> = refs
                .iter()
                .map(|(name, sha)| json!({ "name": name, "commit": { "sha": sha } }))
                .collect();
            let route = format!("/repos/{full_name}/{kind}");
            self.respond("GET", &route, 200, Value::Array(items), &[]);
        }
    }

    /// Answers the branches and the tags of the GitLab project `project` (`(name, sha)`).
    pub fn answer_gitlab_refs(
        &self,
        project: u64,
        branches: &[(&str, &str)],
        tags: &[(&str, &str)],
    ) {
        for (kind, refs) in [("branches", branches), ("tags", tags)] {
            let items: Vec<Value> = refs
                .iter()
                .map(|(name, sha)| json!({ "name": name, "commit": { "id": sha } }))
                .collect();
            let route = format!("/api/v4/projects/{project}/repository/{kind}");
            self.respond("GET", &route, 200, Value::Array(items), &[]);
        }
    }
}
