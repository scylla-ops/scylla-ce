pub mod commands;
pub mod queries;
pub mod repository;
pub mod stream_port;

pub use commands::AppendJobLogs;
pub use queries::{ListJobLogs, TailJobLogs};
pub use repository::JobLogRepository;
pub use stream_port::{JobLogLiveStream, JobLogStreamPort};

use crate::application::JobRepository;
use crate::domain::errors::DomainResult;
use crate::domain::ids::JobId;
use crate::domain::pipeline::NodeId;
use derive_more::Constructor;
use std::sync::Arc;

/// The job log's stage runners, one block per action in `commands.rs` and `queries.rs`. The
/// agent stream sends `AppendJobLogs` through the engine, and its `commit` closure pushes the
/// stored lines to the live tail. `job_repo` serves the placement check of an append and the
/// node rule of a read.
#[derive(Constructor)]
pub struct JobLogUseCases {
    pub(super) log_repo: Arc<dyn JobLogRepository>,
    pub(super) stream_port: Arc<dyn JobLogStreamPort>,
    pub(super) job_repo: Arc<dyn JobRepository>,
}

impl JobLogUseCases {
    /// A line can be stored before the status that starts its node.
    pub(super) async fn node_readable(
        &self,
        job_id: &JobId,
        node_id: Option<&NodeId>,
    ) -> DomainResult<bool> {
        match node_id {
            Some(node_id) => Ok(self
                .job_repo
                .find_by_id(job_id)
                .await?
                .logs_readable_for(node_id)),
            None => Ok(true),
        }
    }
}

#[cfg(test)]
mod tests;
