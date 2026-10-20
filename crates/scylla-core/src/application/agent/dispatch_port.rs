use crate::application::agent::dispatch::JobDispatch;
use crate::domain::errors::DomainResult;
use crate::domain::ids::{AppId, JobId, StreamId};

#[derive(Debug, Clone)]
pub enum AgentOrder {
    Run(JobDispatch),
    Cancel(JobId),
}

/// One open stream of an agent. A job placed on an agent names the stream that took it, and
/// only a release that names that stream returns the job to the pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStream {
    pub agent: AppId,
    pub id: StreamId,
}

/// The open agent streams, one for each agent. `wake` asks the dispatcher for a pass: over one
/// agent, or over every connected agent with `None`. An order never waits: a stream that is
/// gone, or that does not take the order, is an error.
pub trait AgentDispatch: Send + Sync {
    fn connected(&self) -> Vec<AgentStream>;

    /// Only on this stream: a newer stream of the agent does not get the job.
    fn run(&self, stream: &AgentStream, dispatch: JobDispatch) -> DomainResult<()>;

    /// On the open stream of the agent, whichever it is.
    fn cancel(&self, agent: &AppId, job_id: &JobId) -> DomainResult<()>;

    fn disconnect(&self, agent: &AppId);

    fn wake(&self, agent: Option<&AppId>);
}
