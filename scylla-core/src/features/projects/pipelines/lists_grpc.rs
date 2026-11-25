//! The RPCs of `PipelineService` that list the pipelines of a project or of a workspace.

use scylla_domain::projects::pipelines::{
    ListPipelines, PipelineAccess, PipelineHolder, PipelineScope,
};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::pipelines::v1 as pb;
use tonic::{Request, Response, Status};

use super::grpc::PipelinesGrpc;
use super::service;
use crate::api::parse::{self, ToProto};
use crate::Error;

impl PipelinesGrpc {
    /// Answers `ListProjectPipelines`: one page of the pipelines of a project, oldest first.
    pub(super) async fn list_in_project(
        &self,
        request: Request<pb::ListProjectPipelinesRequest>,
    ) -> Result<Response<pb::ListProjectPipelinesResponse>, Status> {
        // Authenticate the caller, read the request, and check the permission on the project.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let project: ProjectId = parse::required(request.project_id, "project_id")?;
        let page = parse::optional(request.page, "page")?;
        self.authorize_in(&caller, PipelineAccess::LIST_PROJECT_PIPELINES, project)
            .await?;

        // Read one page.
        let query = ListPipelines {
            scope: PipelineScope::Project(project),
            page,
        };
        let page = service::list(&self.vcs, query).await?;
        Ok(Response::new(pb::ListProjectPipelinesResponse {
            pipelines: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
        }))
    }

    /// Answers `ListWorkspacePipelines`: one page of the pipelines of each project of a
    /// workspace, oldest first.
    pub(super) async fn list_in_workspace(
        &self,
        request: Request<pb::ListWorkspacePipelinesRequest>,
    ) -> Result<Response<pb::ListWorkspacePipelinesResponse>, Status> {
        // Authenticate the caller, read the request, and check the permission on the workspace.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let workspace: WorkspaceId = parse::required(request.workspace_id, "workspace_id")?;
        let page = parse::optional(request.page, "page")?;
        self.gate
            .authorize(
                &caller,
                PipelineHolder::LIST_WORKSPACE_PIPELINES.on(workspace),
            )
            .await
            .map_err(Error::from)?;

        // Read one page.
        let query = ListPipelines {
            scope: PipelineScope::Workspace(workspace),
            page,
        };
        let page = service::list(&self.vcs, query).await?;
        Ok(Response::new(pb::ListWorkspacePipelinesResponse {
            pipelines: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
        }))
    }
}
