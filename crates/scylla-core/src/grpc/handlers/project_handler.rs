//! The adapter: each RPC is one `run` and its response. Parsing lives in the
//! project mapper, behind `Parse`; no RPC checks a permission or touches a port.

use crate::application::ProjectUseCases;
use crate::grpc::adapter::run;
use crate::grpc::mappers::project_to_proto;
use derive_more::Constructor;
use scylla_extension::Actions;
use scylla_proto::project::v1::{
    CreateProjectRequest, CreateProjectResponse, DeleteProjectRequest, DeleteProjectResponse,
    GetProjectRequest, GetProjectResponse, ListOrganizationProjectsRequest,
    ListOrganizationProjectsResponse, ListProjectMembersRequest, ListProjectMembersResponse,
    ListProjectsRequest, ListProjectsResponse, ListUserProjectsRequest, ListUserProjectsResponse,
    SetProjectActiveRequest, SetProjectActiveResponse, UpdateProjectRequest, UpdateProjectResponse,
    project_service_server::ProjectService,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

#[derive(Constructor)]
pub struct ProjectHandler {
    actions: Arc<Actions>,
    projects: Arc<ProjectUseCases>,
}

#[async_trait::async_trait]
impl ProjectService for ProjectHandler {
    async fn create_project(
        &self,
        request: Request<CreateProjectRequest>,
    ) -> Result<Response<CreateProjectResponse>, Status> {
        let project = run(&self.actions, &*self.projects, request).await?;
        Ok(Response::new(CreateProjectResponse {
            project: Some(project_to_proto(&project)),
        }))
    }

    async fn get_project(
        &self,
        request: Request<GetProjectRequest>,
    ) -> Result<Response<GetProjectResponse>, Status> {
        let project = run(&self.actions, &*self.projects, request).await?;
        Ok(Response::new(GetProjectResponse {
            project: Some(project_to_proto(&project)),
        }))
    }

    async fn update_project(
        &self,
        request: Request<UpdateProjectRequest>,
    ) -> Result<Response<UpdateProjectResponse>, Status> {
        let project = run(&self.actions, &*self.projects, request).await?;
        Ok(Response::new(UpdateProjectResponse {
            project: Some(project_to_proto(&project)),
        }))
    }

    async fn set_project_active(
        &self,
        request: Request<SetProjectActiveRequest>,
    ) -> Result<Response<SetProjectActiveResponse>, Status> {
        let project = run(&self.actions, &*self.projects, request).await?;
        Ok(Response::new(SetProjectActiveResponse {
            project: Some(project_to_proto(&project)),
        }))
    }

    async fn delete_project(
        &self,
        request: Request<DeleteProjectRequest>,
    ) -> Result<Response<DeleteProjectResponse>, Status> {
        run(&self.actions, &*self.projects, request).await?;
        Ok(Response::new(DeleteProjectResponse {}))
    }

    async fn list_projects(
        &self,
        request: Request<ListProjectsRequest>,
    ) -> Result<Response<ListProjectsResponse>, Status> {
        let page = run(&self.actions, &*self.projects, request).await?;
        Ok(Response::new(page.into()))
    }

    async fn list_organization_projects(
        &self,
        request: Request<ListOrganizationProjectsRequest>,
    ) -> Result<Response<ListOrganizationProjectsResponse>, Status> {
        let page = run(&self.actions, &*self.projects, request).await?;
        Ok(Response::new(page.into()))
    }

    async fn list_user_projects(
        &self,
        request: Request<ListUserProjectsRequest>,
    ) -> Result<Response<ListUserProjectsResponse>, Status> {
        let page = run(&self.actions, &*self.projects, request).await?;
        Ok(Response::new(page.into()))
    }

    async fn list_project_members(
        &self,
        request: Request<ListProjectMembersRequest>,
    ) -> Result<Response<ListProjectMembersResponse>, Status> {
        let page = run(&self.actions, &*self.projects, request).await?;
        Ok(Response::new(page.into()))
    }
}
