//! The RPC of `PipelineService` for the opt-in of the Git events of an editor pipeline.

use scylla_domain::projects::pipelines::PipelineAccess;
use scylla_proto::pipelines::v1 as pb;
use tonic::{Request, Response, Status};

use super::git_events_service;
use super::grpc::PipelinesGrpc;
use crate::api::parse::ToProto;
use crate::Error;

impl PipelinesGrpc {
    /// Answers `SetPipelineGitEvents`: lets the Git triggers of an editor pipeline start runs in
    /// a project of files, or stops them. Only the admins of the workspace pass the check.
    pub(super) async fn set_git_events(
        &self,
        request: Request<pb::SetPipelineGitEventsRequest>,
    ) -> Result<Response<pb::SetPipelineGitEventsResponse>, Status> {
        // Authenticate the caller, then check the route permission on the pipeline.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let permission = PipelineAccess::SET_PIPELINE_GIT_EVENTS;
        let id = self
            .pipeline_of(&caller, request.pipeline_id, permission)
            .await?;

        // Change the opt-in.
        let actor = caller.actor.id;
        let pipeline = git_events_service::set(&self.pool, id, actor, request.allowed).await?;
        Ok(Response::new(pb::SetPipelineGitEventsResponse {
            pipeline: Some(pipeline.to_proto()),
        }))
    }
}
