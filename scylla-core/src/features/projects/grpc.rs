//! The gRPC handler of `ProjectService`: authenticate, authorize, parse, call the service, answer.

use std::sync::Arc;

use scylla_domain::projects::{
    ListProjects, NewProject, Project, ProjectCaller, ProjectHolder, ProjectId, Visible,
};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{AccessError, Gate, Hooks};
use scylla_proto::projects::v1 as pb;
use scylla_proto::projects::v1::project_service_server::{ProjectService, ProjectServiceServer};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::service;
use crate::api::parse::{self, ToProto};
use crate::Error;

/// Makes the service.
pub(super) fn server(
    pool: PgPool,
    hooks: Arc<Hooks>,
    gate: Arc<dyn Gate>,
) -> ProjectServiceServer<ProjectsGrpc> {
    ProjectServiceServer::new(ProjectsGrpc { pool, hooks, gate })
}

/// The handler.
pub(super) struct ProjectsGrpc {
    /// The database pool.
    pool: PgPool,
    /// The hooks of the edition.
    hooks: Arc<Hooks>,
    /// The access gate.
    gate: Arc<dyn Gate>,
}

#[tonic::async_trait]
impl ProjectService for ProjectsGrpc {
    async fn create_project(
        &self,
        request: Request<pb::CreateProjectRequest>,
    ) -> Result<Response<pb::CreateProjectResponse>, Status> {
        Ok(Response::new(self.create(request).await?))
    }

    async fn get_project(
        &self,
        request: Request<pb::GetProjectRequest>,
    ) -> Result<Response<pb::GetProjectResponse>, Status> {
        Ok(Response::new(self.get(request).await?))
    }

    async fn list_projects(
        &self,
        request: Request<pb::ListProjectsRequest>,
    ) -> Result<Response<pb::ListProjectsResponse>, Status> {
        Ok(Response::new(self.list(request).await?))
    }

    async fn delete_project(
        &self,
        request: Request<pb::DeleteProjectRequest>,
    ) -> Result<Response<pb::DeleteProjectResponse>, Status> {
        Ok(Response::new(self.delete(request).await?))
    }
}

impl ProjectsGrpc {
    /// Handles `CreateProject`, in a workspace.
    async fn create(
        &self,
        request: Request<pb::CreateProjectRequest>,
    ) -> Result<pb::CreateProjectResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let new = NewProject {
            workspace_id: parse::required(request.workspace_id, "workspace_id")?,
            created_by: caller.actor.id,
            name: parse::field(request.name, "name")?,
        };

        // Check the route permission on the workspace.
        self.gate
            .authorize(&caller, ProjectHolder::CREATE_PROJECT.on(new.workspace_id))
            .await?;

        // Create the project.
        let project = service::create(&self.pool, &self.hooks, caller.actor, new).await?;
        Ok(pb::CreateProjectResponse {
            project: Some(project.to_proto()),
        })
    }

    /// Handles `GetProject`. An unknown project gives `PERMISSION_DENIED`, so no caller learns
    /// which ids exist.
    async fn get(
        &self,
        request: Request<pb::GetProjectRequest>,
    ) -> Result<pb::GetProjectResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let id: ProjectId = parse::required(request.into_inner().project_id, "project_id")?;

        // Check the route permission. An unknown project gets the answer of a refusal.
        let check = Project::GET_PROJECT.on(id);
        let Some(project) = service::get(&self.pool, id).await? else {
            return Err(AccessError::denied(&caller, &check).into());
        };
        self.gate
            .authorize(&caller, check.within(project.workspace_id))
            .await?;
        Ok(pb::GetProjectResponse {
            project: Some(project.to_proto()),
        })
    }

    /// Handles `ListProjects`: the projects that the caller can see, of all their workspaces or
    /// of the workspace of the request.
    async fn list(
        &self,
        request: Request<pb::ListProjectsRequest>,
    ) -> Result<pb::ListProjectsResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let query = ListProjects {
            workspace_id: request
                .workspace_id
                .map(|id| parse::field::<_, WorkspaceId>(id, "workspace_id"))
                .transpose()?,
            page: parse::optional(request.page, "page")?,
        };

        // Check the route permission on the caller: the list shows only what it can see.
        self.gate
            .authorize(&caller, ProjectCaller::LIST_PROJECTS.on(caller.actor.id))
            .await?;

        // Find the workspaces where the caller has a role, and the projects shared with it.
        let (workspaces, shared) = tokio::join!(
            self.gate.granted(&caller, ProjectHolder::LIST_PROJECTS),
            self.gate.granted(&caller, Project::LIST_PROJECTS),
        );
        let visible = Visible {
            caller: caller.actor.id,
            workspaces: or_owned_only(workspaces),
            projects: or_owned_only(shared),
        };

        // Read one page of these projects and of the projects of the workspaces of the caller.
        let page = service::list(&self.pool, query, visible).await?;
        Ok(pb::ListProjectsResponse {
            projects: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
        })
    }

    /// Handles `DeleteProject`. An unknown project gives `PERMISSION_DENIED`, so no caller
    /// learns which ids exist.
    async fn delete(
        &self,
        request: Request<pb::DeleteProjectRequest>,
    ) -> Result<pb::DeleteProjectResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let id: ProjectId = parse::required(request.into_inner().project_id, "project_id")?;

        // Check the route permission. An unknown project gets the answer of a refusal.
        let check = Project::DELETE_PROJECT.on(id);
        let Some(workspace) = service::workspace(&self.pool, id).await? else {
            return Err(AccessError::denied(&caller, &check).into());
        };
        self.gate
            .authorize(&caller, check.within(workspace))
            .await?;

        // Delete the project.
        let gate = self.gate.as_ref();
        service::delete(&self.pool, &self.hooks, gate, caller.actor, id).await?;
        Ok(pb::DeleteProjectResponse {})
    }
}

/// Keeps the ids of a lookup. A failed lookup gives no id: the caller still sees the projects
/// of its workspaces.
fn or_owned_only<T>(lookup: Result<Vec<T>, AccessError>) -> Vec<T> {
    lookup.unwrap_or_else(|error| {
        tracing::warn!(%error, "a lookup failed: the list shows only the owned projects");
        Vec::new()
    })
}

impl ToProto<pb::Project> for Project {
    fn to_proto(self) -> pb::Project {
        pb::Project {
            id: Some(self.id.to_proto()),
            workspace_id: Some(self.workspace_id.to_proto()),
            created_by: Some(self.created_by.to_proto()),
            name: self.name,
            created_at: Some(parse::timestamp(self.created_at)),
        }
    }
}
