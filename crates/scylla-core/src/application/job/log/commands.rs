//! The job log's writes. Each command, its access and its payload types. The strata module of the
//! slice runs it.

use crate::domain::ids::JobId;
use crate::domain::job::JobLog;
use crate::domain::permission::Permission;
use scylla_extension::{Access, Command, Describe, Draft};

/// Consecutive lines of one job from its agent, in the order received: one check and one
/// insert for the batch. Only the agent the job is placed on appends, until the job ends.
#[derive(Debug)]
pub struct AppendJobLogs {
    pub job_id: JobId,
    pub logs: Vec<JobLog>,
}

impl Describe for AppendJobLogs {
    fn access(&self) -> Access {
        Access::Requires(Permission::AppendJobLog(self.job_id.clone()))
    }
}

impl Command for AppendJobLogs {
    type Staged = Draft<Vec<JobLog>>;
    type Committed = Vec<JobLog>;
}
