//! The placement of pending jobs on idle agents, and the orders that stop a job on its agent.
//! Only `DispatchPendingJobs` places a job; a run stores the job and wakes the dispatcher.

pub mod commands;

pub use commands::DispatchPendingJobs;

use crate::application::agent::dispatch_port::{AgentDispatch, AgentStream};
use crate::application::job::JobScope;
use crate::application::{JobLogStreamPort, JobRepository};
use crate::domain::errors::DomainResult;
use crate::domain::ids::{AppId, JobId};
use crate::domain::job::Job;
use derive_more::Constructor;
use std::sync::Arc;
use tracing::debug;

/// The orders that stop a job on its agent, and the wakes of the dispatcher. The dispatch pass
/// itself is `JobControl::dispatch_pending_jobs` of the strata edition.
#[derive(Constructor)]
pub struct DispatchUseCases {
    registry: Arc<dyn AgentDispatch>,
    job_repo: Arc<dyn JobRepository>,
    log_stream: Arc<dyn JobLogStreamPort>,
}

impl DispatchUseCases {
    pub fn wake(&self, agent: Option<&AppId>) {
        self.registry.wake(agent);
    }

    pub fn open_streams(&self) -> Vec<AgentStream> {
        self.registry.connected()
    }

    pub fn disconnect(&self, agent: &AppId) {
        self.registry.disconnect(agent);
    }

    /// Every way a job goes back to the pool: the jobs of a stream that is gone, and the
    /// dispatcher wakes for them.
    pub async fn release(&self, stream: &AgentStream) -> DomainResult<u64> {
        let released = self.job_repo.release(stream).await?;
        if released > 0 {
            self.registry.wake(None);
        }
        Ok(released)
    }

    /// After a server-side end: the agent that holds the job stops it, and the live tail closes.
    pub fn stop(&self, job: &Job) {
        if let Some(agent) = job.agent_app_id() {
            self.cancel(agent, job.id());
        }
        self.log_stream.close(job.id());
    }

    /// An agent that is gone learns it at its next hello.
    pub fn cancel(&self, agent: &AppId, job_id: &JobId) {
        if let Err(e) = self.registry.cancel(agent, job_id) {
            debug!(app_id = %agent, job_id = %job_id, error = %e, "could not send the cancel to the agent");
        }
        self.registry.wake(Some(agent));
    }

    /// Runs a delete that cascades to jobs, then stops the jobs of `scope` that were live; a
    /// delete that fails stops nothing.
    pub async fn recall<T>(
        &self,
        scope: JobScope<'_>,
        delete: impl Future<Output = DomainResult<T>>,
    ) -> DomainResult<T> {
        let live = self.job_repo.list_live(scope).await?;
        let deleted = delete.await?;
        for job in &live {
            self.stop(job);
        }
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests;
