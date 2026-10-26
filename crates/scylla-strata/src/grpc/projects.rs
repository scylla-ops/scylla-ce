use crate::{Projects, ProjectsRpc};
use scylla_core::grpc::{domain_error_to_status, project_to_proto};
use scylla_proto::project::v1 as pb;
use strata_tonic::Grpc;

pub use super::service::project::project_service_client::ProjectServiceClient;
pub use super::service::project::project_service_server::{ProjectService, ProjectServiceServer};
use super::{authed, parse};

strata_tonic::service! {
    impl<A: Projects> ProjectService for Grpc<A> {
        rpc create_project(pb::CreateProjectRequest) -> pb::CreateProjectResponse = |call, req| {
            authed(&call).await?;
            let project = ProjectsRpc::create_project(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::CreateProjectResponse { project: Some(project_to_proto(&project)) })
        }
        rpc get_project(pb::GetProjectRequest) -> pb::GetProjectResponse = |call, req| {
            authed(&call).await?;
            let project = ProjectsRpc::get_project(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::GetProjectResponse { project: Some(project_to_proto(&project)) })
        }
        rpc update_project(pb::UpdateProjectRequest) -> pb::UpdateProjectResponse = |call, req| {
            authed(&call).await?;
            let project = ProjectsRpc::update_project(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::UpdateProjectResponse { project: Some(project_to_proto(&project)) })
        }
        rpc set_project_active(pb::SetProjectActiveRequest) -> pb::SetProjectActiveResponse = |call, req| {
            authed(&call).await?;
            let project = ProjectsRpc::set_project_active(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::SetProjectActiveResponse { project: Some(project_to_proto(&project)) })
        }
        rpc delete_project(pb::DeleteProjectRequest) -> pb::DeleteProjectResponse = |call, req| {
            authed(&call).await?;
            ProjectsRpc::delete_project(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::DeleteProjectResponse {})
        }
        rpc list_projects(pb::ListProjectsRequest) -> pb::ListProjectsResponse = |call, req| {
            authed(&call).await?;
            Ok(ProjectsRpc::list_projects(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?.into())
        }
        rpc list_organization_projects(pb::ListOrganizationProjectsRequest) -> pb::ListOrganizationProjectsResponse = |call, req| {
            authed(&call).await?;
            Ok(ProjectsRpc::list_organization_projects(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?.into())
        }
        rpc list_user_projects(pb::ListUserProjectsRequest) -> pb::ListUserProjectsResponse = |call, req| {
            authed(&call).await?;
            Ok(ProjectsRpc::list_user_projects(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?.into())
        }
        rpc list_project_members(pb::ListProjectMembersRequest) -> pb::ListProjectMembersResponse = |call, req| {
            authed(&call).await?;
            Ok(ProjectsRpc::list_project_members(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?.into())
        }
    }
}
