use std::sync::Arc;

use scylla_domain::projects::{ListProjects, NewProject, Project, ProjectId, ProjectName};
use scylla_domain::users::UserId;
use scylla_extension::ResourceGuard;
use scylla_proto::projects::v1 as pb;
use scylla_proto::projects::v1::project_service_server::{ProjectService, ProjectServiceServer};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::service;
use crate::{convert, Error};

const DEFAULT_PAGE_SIZE: usize = 50;
const MAX_PAGE_SIZE: usize = 100;

pub fn server(pool: PgPool, guard: Arc<dyn ResourceGuard>) -> ProjectServiceServer<ProjectsGrpc> {
    ProjectServiceServer::new(ProjectsGrpc { pool, guard })
}

pub struct ProjectsGrpc {
    pool: PgPool,
    guard: Arc<dyn ResourceGuard>,
}

#[tonic::async_trait]
impl ProjectService for ProjectsGrpc {
    async fn create_project(
        &self,
        request: Request<pb::CreateProjectRequest>,
    ) -> Result<Response<pb::CreateProjectResponse>, Status> {
        Ok(Response::new(self.create(request.into_inner()).await?))
    }

    async fn list_projects(
        &self,
        request: Request<pb::ListProjectsRequest>,
    ) -> Result<Response<pb::ListProjectsResponse>, Status> {
        Ok(Response::new(self.list(request.into_inner()).await?))
    }

    async fn delete_project(
        &self,
        request: Request<pb::DeleteProjectRequest>,
    ) -> Result<Response<pb::DeleteProjectResponse>, Status> {
        Ok(Response::new(self.delete(request.into_inner()).await?))
    }
}

impl ProjectsGrpc {
    async fn create(
        &self,
        request: pb::CreateProjectRequest,
    ) -> Result<pb::CreateProjectResponse, Error> {
        let new = NewProject {
            owner_id: UserId(convert::uuid(&request.owner_id, "owner_id")?),
            name: ProjectName::parse(request.name)?,
        };
        let project = service::create(&self.pool, self.guard.as_ref(), new).await?;
        Ok(pb::CreateProjectResponse {
            project: Some(to_pb(project)),
        })
    }

    async fn list(
        &self,
        request: pb::ListProjectsRequest,
    ) -> Result<pb::ListProjectsResponse, Error> {
        let after = match request.page_token.as_str() {
            "" => None,
            token => Some(ProjectId(convert::uuid(token, "page_token")?)),
        };
        let query = ListProjects {
            owner_id: UserId(convert::uuid(&request.owner_id, "owner_id")?),
            after,
            limit: page_size(request.page_size)?,
        };
        let page = service::list(&self.pool, query).await?;
        Ok(pb::ListProjectsResponse {
            projects: page.projects.into_iter().map(to_pb).collect(),
            next_page_token: page
                .next_after
                .map(|id| id.0.to_string())
                .unwrap_or_default(),
        })
    }

    async fn delete(
        &self,
        request: pb::DeleteProjectRequest,
    ) -> Result<pb::DeleteProjectResponse, Error> {
        let id = ProjectId(convert::uuid(&request.project_id, "project_id")?);
        service::delete(&self.pool, self.guard.as_ref(), id).await?;
        Ok(pb::DeleteProjectResponse {})
    }
}

fn page_size(requested: i32) -> Result<usize, Error> {
    match usize::try_from(requested) {
        Ok(0) => Ok(DEFAULT_PAGE_SIZE),
        Ok(size) => Ok(size.min(MAX_PAGE_SIZE)),
        Err(_) => Err(Error::InvalidArgument {
            field: "page_size",
            reason: "must not be negative",
        }),
    }
}

fn to_pb(project: Project) -> pb::Project {
    pb::Project {
        id: project.id.0.to_string(),
        owner_id: project.owner_id.0.to_string(),
        name: project.name,
        created_at: Some(convert::timestamp(project.created_at)),
    }
}
