//! The gRPC handler of `ProjectService`: authenticate, authorize, parse, call the service, answer.

use std::sync::Arc;

use scylla_domain::actors::ActorId;
use scylla_domain::projects::{ListProjects, NewProject, Project, ProjectId};
use scylla_extension::{Check, Gate, Hooks, Object, Principal};
use scylla_proto::projects::v1 as pb;
use scylla_proto::projects::v1::project_service_server::{ProjectService, ProjectServiceServer};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::service;
use crate::parse::{self, ToProto};
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
    /// Passes if the caller can act for `owner` in this operation.
    async fn act_for(
        &self,
        principal: &Principal,
        operation: &'static str,
        owner: ActorId,
    ) -> Result<(), Error> {
        let object = Object::new("actor", owner);
        let check = Check {
            operation,
            object: &object,
            permission: "act_for",
            at_least_as_fresh: None,
        };
        Ok(self.gate.authorize(principal, check).await?)
    }

    /// Handles `CreateProject`. The owner is the caller if the request names no owner.
    async fn create(
        &self,
        request: Request<pb::CreateProjectRequest>,
    ) -> Result<pb::CreateProjectResponse, Error> {
        let principal = self.gate.authenticate(request.metadata()).await?;
        let request = request.into_inner();
        let new = NewProject {
            owner_id: parse::or(request.owner_id, "owner_id", principal.actor.id)?,
            created_by: principal.actor.id,
            name: parse::field(request.name, "name")?,
        };
        self.act_for(&principal, "projects.create", new.owner_id)
            .await?;
        let gate = self.gate.as_ref();
        let project = service::create(&self.pool, &self.hooks, gate, principal.actor, new).await?;
        Ok(pb::CreateProjectResponse {
            project: Some(project.to_proto()),
        })
    }

    /// Handles `ListProjects`. The owner is the caller if the request names no owner.
    async fn list(
        &self,
        request: Request<pb::ListProjectsRequest>,
    ) -> Result<pb::ListProjectsResponse, Error> {
        let principal = self.gate.authenticate(request.metadata()).await?;
        let request = request.into_inner();
        let query = ListProjects {
            owner_id: parse::or(request.owner_id, "owner_id", principal.actor.id)?,
            page: parse::optional(request.page, "page")?,
        };
        self.act_for(&principal, "projects.list", query.owner_id)
            .await?;
        let page = service::list(&self.pool, query).await?;
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
        let principal = self.gate.authenticate(request.metadata()).await?;
        let id: ProjectId = parse::required(request.into_inner().project_id, "project_id")?;
        let object = service::object(id);
        let check = Check {
            operation: "projects.delete",
            object: &object,
            permission: "delete",
            at_least_as_fresh: None,
        };
        self.gate.authorize(&principal, check).await?;
        let gate = self.gate.as_ref();
        service::delete(&self.pool, &self.hooks, gate, principal.actor, id).await?;
        Ok(pb::DeleteProjectResponse {})
    }
}

impl ToProto<pb::Project> for Project {
    fn to_proto(self) -> pb::Project {
        pb::Project {
            id: Some(self.id.to_proto()),
            owner_id: Some(self.owner_id.to_proto()),
            created_by: Some(self.created_by.to_proto()),
            name: self.name,
            created_at: Some(parse::timestamp(self.created_at)),
        }
    }
}
