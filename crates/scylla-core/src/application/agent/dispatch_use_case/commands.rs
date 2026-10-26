use crate::application::agent::AgentStream;
use crate::domain::ids::AppId;
use crate::domain::job::Job;
use scylla_auth::authz::Visibility;
use scylla_extension::{Access, Command, Describe};

/// One pass: each targeted agent that is connected, idle and holds `executeJob` somewhere gets
/// the oldest pending job it may run, on its open stream. `None` targets every connected agent.
/// `Committed` is the jobs an agent took.
#[derive(Debug)]
pub struct DispatchPendingJobs {
    pub agents: Option<Vec<AppId>>,
}

impl Describe for DispatchPendingJobs {
    fn access(&self) -> Access {
        Access::Authenticated
    }
}

impl Command for DispatchPendingJobs {
    type Staged = Vec<(AgentStream, Visibility)>;
    type Committed = Vec<Job>;
}
