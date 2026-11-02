//! The gRPC handler of `ProjectService`: authenticate, parse, call the service, answer.

use std::sync::Arc;

use scylla_domain::actors::Actor;
use scylla_domain::projects::{ListProjects, NewProject, Project};
use scylla_extension::{Authenticators, Hooks};
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
    authenticators: Arc<Authenticators>,
) -> ProjectServiceServer<ProjectsGrpc> {
    ProjectServiceServer::new(ProjectsGrpc {
        pool,
        hooks,
        authenticators,
    })
}

/// The handler.
pub(super) struct ProjectsGrpc {
    /// The database pool.
    pool: PgPool,
    /// The hooks of the edition.
    hooks: Arc<Hooks>,
    /// The authenticators of the server.
    authenticators: Arc<Authenticators>,
}

#[tonic::async_trait]
impl ProjectService for ProjectsGrpc {
    async fn create_project(
        &self,
        request: Request<pb::CreateProjectRequest>,
    ) -> Result<Response<pb::CreateProjectResponse>, Status> {
        let actor = self.actor(&request).await?;
        Ok(Response::new(
            self.create(actor, request.into_inner()).await?,
        ))
    }

    async fn list_projects(
        &self,
        request: Request<pb::ListProjectsRequest>,
    ) -> Result<Response<pb::ListProjectsResponse>, Status> {
        let actor = self.actor(&request).await?;
        Ok(Response::new(self.list(actor, request.into_inner()).await?))
    }

    async fn delete_project(
        &self,
        request: Request<pb::DeleteProjectRequest>,
    ) -> Result<Response<pb::DeleteProjectResponse>, Status> {
        let actor = self.actor(&request).await?;
        Ok(Response::new(
            self.delete(actor, request.into_inner()).await?,
        ))
    }
}

impl ProjectsGrpc {
    /// Returns the actor of a request.
    async fn actor<M>(&self, request: &Request<M>) -> Result<Actor, Error> {
        Ok(self
            .authenticators
            .authenticate(&self.pool, request.metadata())
            .await?)
    }

    /// Handles `CreateProject`. The owner is the caller if the request names no owner.
    async fn create(
        &self,
        actor: Actor,
        request: pb::CreateProjectRequest,
    ) -> Result<pb::CreateProjectResponse, Error> {
        let new = NewProject {
            owner_id: parse::or(request.owner_id, "owner_id", actor.id)?,
            created_by: actor.id,
            name: parse::field(request.name, "name")?,
        };
        let project = service::create(&self.pool, &self.hooks, actor, new).await?;
        Ok(pb::CreateProjectResponse {
            project: Some(project.to_proto()),
        })
    }

    /// Handles `ListProjects`. The owner is the caller if the request names no owner.
    async fn list(
        &self,
        actor: Actor,
        request: pb::ListProjectsRequest,
    ) -> Result<pb::ListProjectsResponse, Error> {
        let query = ListProjects {
            owner_id: parse::or(request.owner_id, "owner_id", actor.id)?,
            page: parse::optional(request.page, "page")?,
        };
        let page = service::list(&self.pool, query).await?;
        Ok(pb::ListProjectsResponse {
            projects: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
        })
    }

    /// Handles `DeleteProject`.
    async fn delete(
        &self,
        actor: Actor,
        request: pb::DeleteProjectRequest,
    ) -> Result<pb::DeleteProjectResponse, Error> {
        let id = parse::required(request.project_id, "project_id")?;
        service::delete(&self.pool, &self.hooks, actor, id).await?;
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
