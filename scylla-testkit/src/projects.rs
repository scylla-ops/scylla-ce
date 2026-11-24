//! The `TestApp` helpers of `ProjectService` and of `SecretService`.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::common::v1 as common;
use scylla_proto::projects::v1::secret_service_client::SecretServiceClient;
use scylla_proto::projects::v1::{
    CreateProjectRequest, CreateSecretRequest, CreateSecretResponse, DeleteProjectRequest,
    DeleteSecretRequest, GetProjectRequest, ListProjectsRequest, ListProjectsResponse,
    ListSecretsRequest, ListSecretsResponse, Project, UpdateProjectRequest, UpdateSecretRequest,
    UpdateSecretResponse,
};
use tonic::transport::Channel;
use tonic::Status;

use crate::rpc::page;
use crate::{id, TestApp};

impl TestApp {
    /// Calls `CreateProject` as `actor`, in the personal workspace of the actor.
    pub async fn create_project(&self, actor: ActorId, name: &str) -> Result<Project, Status> {
        let workspace = self.workspace_of(actor).await;
        self.create_project_in(actor, workspace, name).await
    }

    /// Calls `CreateProject` as `actor`, in `workspace`, with no description.
    pub async fn create_project_in(
        &self,
        actor: ActorId,
        workspace: WorkspaceId,
        name: &str,
    ) -> Result<Project, Status> {
        let request = CreateProjectRequest {
            workspace_id: Some(id(workspace.to_string())),
            name: name.to_owned(),
            description: String::new(),
        };
        let request = self.authed(actor, request).await;
        let response = self.projects.clone().create_project(request).await?;
        Ok(response.into_inner().project.expect("project in response"))
    }

    /// Counts the projects of a workspace in the database.
    pub async fn project_count(&self, workspace: WorkspaceId) -> i64 {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "count!" FROM projects WHERE workspace_id = $1"#,
            workspace.to_db(),
        )
        .fetch_one(&self.pool)
        .await
        .expect("count projects")
    }

    /// Calls `GetProject` as `actor`. The id is text, so a test can send a bad id.
    pub async fn get_project(&self, actor: ActorId, project_id: &str) -> Result<Project, Status> {
        let request = GetProjectRequest {
            project_id: Some(id(project_id)),
        };
        let request = self.authed(actor, request).await;
        let response = self.projects.clone().get_project(request).await?;
        Ok(response.into_inner().project.expect("project in response"))
    }

    /// Calls `ListProjects` as `actor`, for all the projects that the actor can see.
    pub async fn list_projects(
        &self,
        actor: ActorId,
        size: i32,
        token: &str,
    ) -> Result<ListProjectsResponse, Status> {
        let request = ListProjectsRequest {
            workspace_id: None,
            page: Some(page(size, token)),
        };
        let request = self.authed(actor, request).await;
        Ok(self
            .projects
            .clone()
            .list_projects(request)
            .await?
            .into_inner())
    }

    /// Calls `ListProjects` as `actor` for one page of 100, of one workspace or of all, and
    /// returns the names of the projects in list order.
    pub async fn visible_projects(
        &self,
        actor: ActorId,
        workspace: Option<WorkspaceId>,
    ) -> Result<Vec<String>, Status> {
        let request = ListProjectsRequest {
            workspace_id: workspace.map(|workspace| id(workspace.to_string())),
            page: Some(common::PageRequest {
                size: 100,
                token: String::new(),
            }),
        };
        let request = self.authed(actor, request).await;
        let response = self.projects.clone().list_projects(request).await?;
        let projects = response.into_inner().projects;
        Ok(projects.into_iter().map(|project| project.name).collect())
    }

    /// Calls `UpdateProject` as `actor`. A field that is `None` does not change.
    pub async fn update_project(
        &self,
        actor: ActorId,
        project_id: &str,
        name: Option<&str>,
        description: Option<&str>,
    ) -> Result<Project, Status> {
        let request = UpdateProjectRequest {
            project_id: Some(id(project_id)),
            name: name.map(str::to_owned),
            description: description.map(str::to_owned),
        };
        let request = self.authed(actor, request).await;
        let response = self.projects.clone().update_project(request).await?;
        Ok(response.into_inner().project.expect("project in response"))
    }

    /// Calls `DeleteProject` as `actor`. The id is text, so a test can send a bad id.
    pub async fn delete_project(&self, actor: ActorId, project_id: &str) -> Result<(), Status> {
        let request = DeleteProjectRequest {
            project_id: Some(id(project_id)),
        };
        let request = self.authed(actor, request).await;
        self.projects.clone().delete_project(request).await?;
        Ok(())
    }

    /// Calls `CreateSecret` as `actor`, in a project, and returns the whole answer.
    pub async fn create_secret(
        &self,
        actor: ActorId,
        project_id: &str,
        name: &str,
        value: &str,
    ) -> Result<CreateSecretResponse, Status> {
        let request = CreateSecretRequest {
            project_id: Some(id(project_id)),
            name: name.to_owned(),
            value: value.to_owned(),
        };
        let request = self.authed(actor, request).await;
        Ok(self.secrets().create_secret(request).await?.into_inner())
    }

    /// Calls `UpdateSecret` as `actor`, and returns the whole answer.
    pub async fn update_secret(
        &self,
        actor: ActorId,
        secret_id: &str,
        value: &str,
    ) -> Result<UpdateSecretResponse, Status> {
        let request = UpdateSecretRequest {
            secret_id: Some(id(secret_id)),
            value: value.to_owned(),
        };
        let request = self.authed(actor, request).await;
        Ok(self.secrets().update_secret(request).await?.into_inner())
    }

    /// Calls `DeleteSecret` as `actor`.
    pub async fn delete_secret(&self, actor: ActorId, secret_id: &str) -> Result<(), Status> {
        let request = DeleteSecretRequest {
            secret_id: Some(id(secret_id)),
        };
        let request = self.authed(actor, request).await;
        self.secrets().delete_secret(request).await?;
        Ok(())
    }

    /// Calls `ListSecrets` as `actor` for one page of a project.
    pub async fn list_secrets(
        &self,
        actor: ActorId,
        project_id: &str,
        size: i32,
        token: &str,
    ) -> Result<ListSecretsResponse, Status> {
        let request = ListSecretsRequest {
            project_id: Some(id(project_id)),
            page: Some(page(size, token)),
        };
        let request = self.authed(actor, request).await;
        Ok(self.secrets().list_secrets(request).await?.into_inner())
    }

    /// Calls `ListSecrets` as `actor` for one page of 100, and returns the names in list order.
    pub async fn secret_names(
        &self,
        actor: ActorId,
        project_id: &str,
    ) -> Result<Vec<String>, Status> {
        let page = self.list_secrets(actor, project_id, 100, "").await?;
        Ok(page.secrets.into_iter().map(|secret| secret.name).collect())
    }

    /// Returns a client of `SecretService`.
    fn secrets(&self) -> SecretServiceClient<Channel> {
        SecretServiceClient::new(self.channel.clone())
    }
}
