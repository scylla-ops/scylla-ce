//! The job's writes. Each command, its access and its payload types. The strata module of the slice
//! runs it. Every write of a job is versioned; a pass skips a job that changed since it read it
//! (`Stale`).

use crate::application::agent::AgentStream;
use crate::application::job::JobEvent;
use crate::domain::ids::{AppId, JobId, StreamId};
use crate::domain::job::Job;
use crate::domain::permission::Permission;
use chrono::{DateTime, TimeDelta, Utc};
use scylla_extension::{Access, Command, Deleted, Describe, Draft};

/// One `WriteJobStatus` check; the reads and the write go through the port, so an agent needs
/// no `readJob`. Only the agent the job is placed on reports it, until the job ends. `at` is
/// the agent's time of the event, kept between the creation of the job and now.
#[derive(Debug)]
pub struct RecordJobStatus {
    pub job_id: JobId,
    pub event: JobEvent,
    pub at: DateTime<Utc>,
}

impl Describe for RecordJobStatus {
    fn access(&self) -> Access {
        Access::Requires(Permission::WriteJobStatus(self.job_id.clone()))
    }
}

impl Command for RecordJobStatus {
    type Staged = Draft<Job>;
    type Committed = Job;
}

#[derive(Debug)]
pub struct CancelJob {
    pub id: JobId,
}

impl Describe for CancelJob {
    fn access(&self) -> Access {
        Access::Requires(Permission::UpdateJob(self.id.clone()))
    }
}

impl Command for CancelJob {
    type Staged = Draft<Job>;
    type Committed = Job;
}

#[derive(Debug)]
pub struct DeleteJob {
    pub id: JobId,
}

impl Describe for DeleteJob {
    fn access(&self) -> Access {
        Access::Requires(Permission::DeleteJob(self.id.clone()))
    }
}

impl Command for DeleteJob {
    type Staged = Job;
    type Committed = Deleted<Job>;
}

/// How long a running job stays with an agent that is not connected, after its last contact.
pub const ORPHAN_GRACE: TimeDelta = TimeDelta::seconds(60);

/// One pass: a running job becomes orphaned when its agent is not connected and was not seen
/// for `ORPHAN_GRACE`, or at once when its agent row is gone. The jobs placed on a stream that
/// is not open go back to the pool. The open streams are read after the jobs: a stream opens
/// before it holds a job, so a stream that the pass finds closed is gone for good.
/// `Committed` is the number of jobs changed.
#[derive(Debug)]
pub struct ReapOrphanedJobs;

impl Describe for ReapOrphanedJobs {
    fn access(&self) -> Access {
        Access::Authenticated
    }
}

impl Command for ReapOrphanedJobs {
    type Staged = (Vec<Draft<Job>>, Vec<AgentStream>);
    type Committed = u64;
}

/// Sent by the agent stream as the agent when the stream ends: the jobs placed on the stream
/// that have not started go back to the pool. `Committed` is their count.
#[derive(Debug)]
pub struct ReleaseAgentJobs {
    pub stream: StreamId,
}

impl Describe for ReleaseAgentJobs {
    fn access(&self) -> Access {
        Access::Authenticated
    }
}

impl Command for ReleaseAgentJobs {
    type Staged = AgentStream;
    type Committed = u64;
}

/// Sent by the agent stream after each hello, as the agent. `running` is the list of jobs the
/// agent runs: a running job of the agent that the list omits becomes orphaned, a listed
/// running job keeps its live tail across a restart of the server, and a listed job that does
/// not run on the agent gets a cancel. `Committed` is the number of jobs changed.
#[derive(Debug)]
pub struct ReconcileAgentJobs {
    pub running: Vec<JobId>,
}

impl Describe for ReconcileAgentJobs {
    fn access(&self) -> Access {
        Access::Authenticated
    }
}

impl Command for ReconcileAgentJobs {
    type Staged = (AppId, Vec<Draft<Job>>, Vec<JobId>, Vec<JobId>);
    type Committed = u64;
}
