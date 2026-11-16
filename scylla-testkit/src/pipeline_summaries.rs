//! The `TestApp` helper of the summaries of the pipelines for the dashboard.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::pipelines::v1::{ListPipelineSummariesRequest, ListPipelineSummariesResponse};
use tonic::Status;

use crate::TestApp;

impl TestApp {
    /// Calls `ListPipelineSummaries` as `actor`.
    pub async fn list_pipeline_summaries(
        &self,
        actor: ActorId,
        request: ListPipelineSummariesRequest,
    ) -> Result<ListPipelineSummariesResponse, Status> {
        let request = self.authed(actor, request).await;
        Ok(self
            .pipelines()
            .list_pipeline_summaries(request)
            .await?
            .into_inner())
    }
}
