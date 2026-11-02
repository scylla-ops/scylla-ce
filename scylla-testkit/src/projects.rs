//! The `TestApp` helpers of `ProjectService`.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::common::v1 as common;
use scylla_proto::projects::v1::{
    CreateProjectRequest, DeleteProjectRequest, GetProjectRequest, ListProjectsRequest,
    ListProjectsResponse, Project,
};
use tonic::Status;

use crate::{id, TestApp};

impl TestApp {
    /// Calls `CreateProject` as `actor`, in the personal workspace of the actor.
    pub async fn create_project(&self, actor: ActorId, name: &str) -> Result<Project, Status> {
        let workspace = self.workspace_of(actor).await;
        self.create_project_in(actor, workspace, name).await
    }

    /// Calls `CreateProject` as `actor`, in `workspace`.
    pub async fn create_project_in(
        &self,
        actor: ActorId,
        workspace: WorkspaceId,
        name: &str,
    ) -> Result<Project, Status> {
        let request = CreateProjectRequest {
            workspace_id: Some(id(workspace.to_string())),
            name: name.to_owned(),
        };
        let request = self.authed(actor, request).await;
        let response = self.projects.clone().create_project(request).await?;
        Ok(response.into_inner().project.expect("project in response"))
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
            page: Some(common::PageRequest {
                size,
                token: token.to_owned(),
            }),
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

    /// Calls `DeleteProject` as `actor`. The id is text, so a test can send a bad id.
    pub async fn delete_project(&self, actor: ActorId, project_id: &str) -> Result<(), Status> {
        let request = DeleteProjectRequest {
            project_id: Some(id(project_id)),
        };
        let request = self.authed(actor, request).await;
        self.projects.clone().delete_project(request).await?;
        Ok(())
    }
}
