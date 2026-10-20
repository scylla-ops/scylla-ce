//! The placement of pending jobs on idle agents, and the orders that stop a job on its agent.
//! Only `DispatchPendingJobs` places a job; a run stores the job and wakes the dispatcher.

pub mod commands;

pub use commands::DispatchPendingJobs;

use crate::application::agent::dispatch::assemble_dispatch;
use crate::application::agent::dispatch_port::{AgentDispatch, AgentStream};
use crate::application::job::JobScope;
use crate::application::{JobLogStreamPort, JobRepository, SecretResolver};
use crate::domain::clock;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{AppId, JobId};
use crate::domain::job::Job;
use derive_more::Constructor;
use scylla_auth::authz::{Visibility, VisibilityResolver};
use std::sync::Arc;
use tracing::{debug, warn};

/// The stage runner of `DispatchPendingJobs`. `visibility` reads the grants of an agent to
/// choose its jobs; it asks the access model nothing, so a pass writes no audit row.
#[derive(Constructor)]
pub struct DispatchUseCases {
    registry: Arc<dyn AgentDispatch>,
    visibility: Arc<dyn VisibilityResolver>,
    job_repo: Arc<dyn JobRepository>,
    secret_resolver: Arc<dyn SecretResolver>,
    log_stream: Arc<dyn JobLogStreamPort>,
}

impl DispatchUseCases {
    pub(crate) fn wake(&self, agent: Option<&AppId>) {
        self.registry.wake(agent);
    }

    pub(crate) fn open_streams(&self) -> Vec<AgentStream> {
        self.registry.connected()
    }

    pub(crate) fn disconnect(&self, agent: &AppId) {
        self.registry.disconnect(agent);
    }

    /// Every way a job goes back to the pool: the jobs of a stream that is gone, and the
    /// dispatcher wakes for them.
    pub(crate) async fn release(&self, stream: &AgentStream) -> DomainResult<u64> {
        let released = self.job_repo.release(stream).await?;
        if released > 0 {
            self.registry.wake(None);
        }
        Ok(released)
    }

    /// After a server-side end: the agent that holds the job stops it, and the live tail closes.
    pub(crate) fn stop(&self, job: &Job) {
        if let Some(agent) = job.agent_app_id() {
            self.cancel(agent, job.id());
        }
        self.log_stream.close(job.id());
    }

    /// An agent that is gone learns it at its next hello.
    pub(crate) fn cancel(&self, agent: &AppId, job_id: &JobId) {
        if let Err(e) = self.registry.cancel(agent, job_id) {
            debug!(app_id = %agent, job_id = %job_id, error = %e, "could not send the cancel to the agent");
        }
        self.registry.wake(Some(agent));
    }

    /// Runs a delete that cascades to jobs, then stops the jobs of `scope` that were live; a
    /// delete that fails stops nothing.
    pub(crate) async fn recall<T>(
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

    /// Claims the oldest job the agent may run and sends it on the stream. A job that no
    /// longer assembles fails, and the agent gets the next one.
    async fn place(&self, stream: &AgentStream, visible: &Visibility) -> DomainResult<Option<Job>> {
        while let Some((job, project_id)) = self.job_repo.claim_next(stream, visible).await? {
            match assemble_dispatch(&*self.secret_resolver, &project_id, &job).await {
                Ok(dispatch) => {
                    if let Err(e) = self.registry.run(stream, dispatch) {
                        warn!(app_id = %stream.agent, job_id = %job.id(), error = %e, "could not send the job; it goes back to the pool");
                        self.release(stream).await?;
                        return Ok(None);
                    }
                    return Ok(Some(job));
                }
                Err(e) => {
                    warn!(job_id = %job.id(), error = %e, "the job cannot be dispatched; it fails");
                    match self.job_repo.update(&job.fail(clock::now())?).await {
                        Ok(_) | Err(DomainError::Stale(_)) => {}
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests;
