//! The `TestApp` helper of the figures of the jobs for the dashboard.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::pipelines::v1::{GetJobStatsRequest, GetJobStatsResponse};
use tonic::Status;

use crate::TestApp;

impl TestApp {
    /// Calls `GetJobStats` as `actor`.
    pub async fn get_job_stats(
        &self,
        actor: ActorId,
        request: GetJobStatsRequest,
    ) -> Result<GetJobStatsResponse, Status> {
        let request = self.authed(actor, request).await;
        Ok(self.jobs().get_job_stats(request).await?.into_inner())
    }
}
