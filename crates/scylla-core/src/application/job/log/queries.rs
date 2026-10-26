//! The job log's reads. Each query, its access and its output type. The strata module of the slice
//! runs it.

use crate::application::JobLogLiveStream;
use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::domain::ids::JobId;
use crate::domain::job::JobLog;
use crate::domain::permission::Permission;
use crate::domain::pipeline::NodeId;
use scylla_extension::{Access, Describe, Query};

#[derive(Debug)]
pub struct ListJobLogs {
    pub job_id: JobId,
    pub node_id: Option<NodeId>,
    pub pagination: Option<PaginationParams>,
}

impl Describe for ListJobLogs {
    fn access(&self) -> Access {
        let logs = Permission::ReadJobLogs(self.job_id.clone());
        match self.node_id {
            Some(_) => Access::RequiresAll(vec![logs, Permission::ReadJob(self.job_id.clone())]),
            None => Access::Requires(logs),
        }
    }
}

impl Query for ListJobLogs {
    type Output = PaginatedResult<JobLog>;
}

/// The persisted lines, then the live ones; a live line already in the snapshot is dropped. A
/// node that has not started replays nothing.
#[derive(Debug)]
pub struct TailJobLogs {
    pub job_id: JobId,
    pub node_id: Option<NodeId>,
}

impl Describe for TailJobLogs {
    fn access(&self) -> Access {
        let logs = Permission::ReadJobLogs(self.job_id.clone());
        match self.node_id {
            Some(_) => Access::RequiresAll(vec![logs, Permission::ReadJob(self.job_id.clone())]),
            None => Access::Requires(logs),
        }
    }
}

impl Query for TailJobLogs {
    type Output = JobLogLiveStream;
}
