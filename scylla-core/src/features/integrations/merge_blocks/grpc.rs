//! The gRPC handler of `MergeBlockService`: authenticate, parse, authorize on the project, call
//! the service, answer. The readers of a project list its rules, and the editors change them.

use scylla_domain::authz::Permission;
use scylla_domain::integrations::links::RepositoryAccess;
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::{Project, ProjectId};
use scylla_extension::{AccessError, Principal};
use scylla_proto::integrations::v1 as pb;
use scylla_proto::integrations::v1::merge_block_service_server::{
    MergeBlockService, MergeBlockServiceServer,
};
use tonic::{Request, Response, Status};

use super::service;
use crate::api::parse::{self, ToProto};
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// Makes the service.
pub(super) fn server(ctx: IntegrationContext) -> MergeBlockServiceServer<MergeBlocksGrpc> {
    MergeBlockServiceServer::new(MergeBlocksGrpc { ctx })
}

/// The handler.
pub(super) struct MergeBlocksGrpc {
    /// What the parts of the integration share.
    ctx: IntegrationContext,
}

#[tonic::async_trait]
impl MergeBlockService for MergeBlocksGrpc {
    async fn list_merge_blocks(
        &self,
        request: Request<pb::ListMergeBlocksRequest>,
    ) -> Result<Response<pb::ListMergeBlocksResponse>, Status> {
        let caller = self.ctx.gate.caller(&request).await.map_err(Error::from)?;
        let project = parse::required(request.into_inner().project_id, "project_id")?;
        let permission = RepositoryAccess::LIST_MERGE_BLOCKS;
        self.authorize(&caller, permission, project).await?;
        let blocks = service::list(&self.ctx, project).await?;
        Ok(Response::new(pb::ListMergeBlocksResponse {
            blocks: blocks.into_iter().map(ToProto::to_proto).collect(),
        }))
    }

    async fn set_merge_block(
        &self,
        request: Request<pb::SetMergeBlockRequest>,
    ) -> Result<Response<pb::SetMergeBlockResponse>, Status> {
        // Authenticate the caller, then read the request.
        let caller = self.ctx.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let project = parse::required(request.project_id, "project_id")?;
        let pipeline: Option<PipelineId> = request
            .pipeline_id
            .map(|id| parse::field(id, "pipeline_id"))
            .transpose()?;

        // Check the route permission, then change the rule.
        let permission = RepositoryAccess::SET_MERGE_BLOCK;
        self.authorize(&caller, permission, project).await?;
        let actor = caller.actor.id;
        let block = service::set(&self.ctx, project, pipeline, request.enabled, actor).await?;
        Ok(Response::new(pb::SetMergeBlockResponse {
            block: Some(block.to_proto()),
        }))
    }
}

impl MergeBlocksGrpc {
    /// Checks a route permission on a project. An unknown project gets the answer of a refusal.
    async fn authorize(
        &self,
        caller: &Principal,
        permission: Permission<Project>,
        project: ProjectId,
    ) -> Result<(), Error> {
        let check = permission.on(project);
        let Some(workspace) = service::workspace(&self.ctx, project).await? else {
            return Err(AccessError::denied(caller, &check).into());
        };
        self.ctx
            .gate
            .authorize(caller, check.within(workspace))
            .await?;
        Ok(())
    }
}
