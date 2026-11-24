//! The approval of a job from a fork: `ApproveJob` and `RejectJob` of `JobService`.

use scylla_domain::projects::pipelines::PipelineAccess;
use scylla_proto::pipelines::v1 as pb;
use tonic::{Request, Response, Status};

use super::grpc::JobsGrpc;
use super::{convert, service};
use crate::Error;

impl JobsGrpc {
    /// Answers `ApproveJob`: a job from a fork that waits becomes pending.
    pub(super) async fn approve(
        &self,
        request: Request<pb::ApproveJobRequest>,
    ) -> Result<Response<pb::ApproveJobResponse>, Status> {
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let id = request.into_inner().job_id;
        let id = self
            .job_of(&caller, id, PipelineAccess::APPROVE_JOB)
            .await?;
        let job = service::approve(&self.pool, &self.registry, caller.actor, id).await?;
        Ok(Response::new(pb::ApproveJobResponse {
            job: Some(convert::job_of(job, &self.registry)),
        }))
    }

    /// Answers `RejectJob`: a job from a fork that waits is cancelled.
    pub(super) async fn reject(
        &self,
        request: Request<pb::RejectJobRequest>,
    ) -> Result<Response<pb::RejectJobResponse>, Status> {
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let id = request.into_inner().job_id;
        let id = self.job_of(&caller, id, PipelineAccess::REJECT_JOB).await?;
        let job = service::reject(&self.pool, caller.actor, id).await?;
        Ok(Response::new(pb::RejectJobResponse {
            job: Some(convert::job_of(job, &self.registry)),
        }))
    }
}
