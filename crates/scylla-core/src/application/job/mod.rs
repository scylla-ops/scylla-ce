pub mod commands;
pub mod log;
pub mod queries;
pub mod reaper;
pub mod repository;

pub use scylla_domain::JobEvent;

pub use commands::{
    CancelJob, DeleteJob, ORPHAN_GRACE, ReapOrphanedJobs, ReconcileAgentJobs, RecordJobStatus,
    ReleaseAgentJobs,
};
pub use log::{
    AppendJobLogs, JobLogLiveStream, JobLogRepository, JobLogStreamPort, JobLogUseCases,
    ListJobLogs, TailJobLogs,
};
pub use queries::{GetJob, ListJobs, ListOrganizationJobs, ListPipelineJobs, ListProjectJobs};
pub use reaper::JobReaper;
pub use repository::{JobRepository, JobScope};

use crate::application::DispatchUseCases;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::job::Job;
use derive_more::Constructor;
use scylla_extension::Draft;
use std::sync::Arc;

/// The job's stage runners, one block per action in `commands.rs` and `queries.rs`. The agent
/// stream sends `RecordJobStatus`, `ReleaseAgentJobs` and `ReconcileAgentJobs`, and `JobReaper`
/// sends `ReapOrphanedJobs`. A job that ends on the server is stopped on its agent through
/// `dispatch`.
#[derive(Constructor)]
pub struct JobUseCases {
    pub(super) job_repo: Arc<dyn JobRepository>,
    pub(super) log_stream: Arc<dyn JobLogStreamPort>,
    pub(super) dispatch: Arc<DispatchUseCases>,
}

impl JobUseCases {
    /// Writes each job a pass orphaned and stops it on its agent. A job that changed since the
    /// pass read it is skipped.
    async fn orphan_all(&self, orphans: Vec<Draft<Job>>) -> DomainResult<u64> {
        let mut written = 0;
        for draft in orphans {
            match self.job_repo.update(&draft.into_inner()).await {
                Ok(job) => self.dispatch.stop(&job),
                Err(DomainError::Stale(_)) => continue,
                Err(e) => return Err(e),
            }
            written += 1;
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests;
