//! The route checks of the handler of `PipelineService`: on a project, and on a pipeline.

use scylla_domain::authz::Permission;
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Principal;

use super::grpc::PipelinesGrpc;
use super::{access, service};
use crate::api::parse;
use crate::Error;

impl PipelinesGrpc {
    /// Checks a route permission on a project and on its workspace, and returns the workspace.
    /// An unknown project gets the answer of a refusal.
    pub(super) async fn authorize_in(
        &self,
        caller: &Principal,
        permission: Permission<Project>,
        id: ProjectId,
    ) -> Result<WorkspaceId, Error> {
        let workspace = service::project_workspace(&self.pool, id).await?;
        let place = workspace.map(|workspace| (id, workspace));
        let object = format!("project:{id}");
        let (_, workspace) =
            access::authorize(self.gate.as_ref(), caller, permission, object, place).await?;
        Ok(workspace)
    }

    /// Reads the pipeline id of a request and checks a route permission on its project. An
    /// unknown pipeline gets the answer of a refusal.
    pub(super) async fn pipeline_of(
        &self,
        caller: &Principal,
        id: Option<scylla_proto::common::v1::Id>,
        permission: Permission<Project>,
    ) -> Result<PipelineId, Error> {
        let id: PipelineId = parse::required(id, "pipeline_id")?;
        let place = service::place(&self.pool, id).await?;
        let object = format!("pipeline:{id}");
        access::authorize(self.gate.as_ref(), caller, permission, object, place).await?;
        Ok(id)
    }
}
