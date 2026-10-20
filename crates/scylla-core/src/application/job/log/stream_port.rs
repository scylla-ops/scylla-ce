use crate::domain::errors::DomainResult;
use crate::domain::ids::JobId;
use crate::domain::job::JobLog;
use crate::domain::pipeline::NodeId;
use async_trait::async_trait;
use futures_core::Stream;
use std::pin::Pin;

pub type JobLogLiveStream = Pin<Box<dyn Stream<Item = DomainResult<JobLog>> + Send + Sync>>;

/// The live tail of a job: `open` when it starts, `publish` each stored line, `close` when it
/// ends. `subscribe` never opens a tail.
#[async_trait]
pub trait JobLogStreamPort: Send + Sync {
    fn open(&self, job_id: &JobId);

    fn publish(&self, log: &JobLog);

    fn close(&self, job_id: &JobId);

    async fn subscribe(
        &self,
        job_id: &JobId,
        node_id: Option<&NodeId>,
    ) -> DomainResult<JobLogLiveStream>;
}
