//! The approval of a job from a fork: `ApproveJob` and `RejectJob` of `JobService`.

use scylla_proto::pipelines::v1 as pb;
use tonic::{Request, Response, Status};

use super::grpc::JobsGrpc;
use crate::Error;

impl JobsGrpc {
    /// Answers `ApproveJob`: the server does not serve it.
    pub(super) async fn approve(
        &self,
        request: Request<pb::ApproveJobRequest>,
    ) -> Result<Response<pb::ApproveJobResponse>, Status> {
        self.gate.caller(&request).await.map_err(Error::from)?;
        Err(Status::unimplemented("ApproveJob is not served"))
    }

    /// Answers `RejectJob`: the server does not serve it.
    pub(super) async fn reject(
        &self,
        request: Request<pb::RejectJobRequest>,
    ) -> Result<Response<pb::RejectJobResponse>, Status> {
        self.gate.caller(&request).await.map_err(Error::from)?;
        Err(Status::unimplemented("RejectJob is not served"))
    }
}
