//! The job's writes. One block per command, in the order it runs: the struct, its access,
//! its payload types, what `Prepare` builds, what `Persist` writes. Every write of a job is
//! versioned; a pass skips a job that changed since it read it (`Stale`).

use super::JobUseCases;
use crate::application::actions::{app_only, service_only};
use crate::application::agent::AgentStream;
use crate::application::job::JobEvent;
use crate::domain::clock;
use crate::domain::errors::DomainResult;
use crate::domain::ids::{AppId, JobId, StreamId};
use crate::domain::job::{Job, NodeOutcome};
use crate::domain::permission::Permission;
use crate::domain::pipeline::NodeId;
use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use scylla_extension::{
    Access, Authorized, Command, Committed, Deleted, Describe, Draft, Persist, Prepare, Prepared,
    Run,
};

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

#[async_trait]
impl Run<Prepare<RecordJobStatus>> for JobUseCases {
    async fn run(
        &self,
        input: Authorized<RecordJobStatus>,
    ) -> DomainResult<Prepared<RecordJobStatus>> {
        let agent = app_only(input.caller())?;
        let cmd = input.command();
        let job = self.job_repo.find_by_id(&cmd.job_id).await?;
        job.ensure_live_on(&agent)?;
        let at = cmd.at.max(job.created_at()).min(clock::now());
        let job = match &cmd.event {
            JobEvent::JobStarted => job.start(at)?,
            JobEvent::NodeStarted { node_id } => {
                job.apply_node_started(&NodeId::new(node_id)?, at)?
            }
            JobEvent::NodeCompleted { node_id } => {
                job.apply_node_finished(&NodeId::new(node_id)?, NodeOutcome::Completed, at)?
            }
            JobEvent::NodeFailed { node_id, .. } => {
                job.apply_node_finished(&NodeId::new(node_id)?, NodeOutcome::Failed, at)?
            }
            JobEvent::NodeSkipped { node_id } => {
                job.apply_node_skipped(&NodeId::new(node_id)?, at)?
            }
            JobEvent::JobCompleted => job.complete(at)?,
            JobEvent::JobFailed { .. } => job.fail(at)?,
        };
        Ok(input.prepared(Draft::new(job)))
    }
}

#[async_trait]
impl Run<Persist<RecordJobStatus>> for JobUseCases {
    async fn run(
        &self,
        input: Prepared<RecordJobStatus>,
    ) -> DomainResult<Committed<RecordJobStatus>> {
        let started = matches!(input.command().event, JobEvent::JobStarted);
        input
            .commit(async |draft| {
                let job = self.job_repo.update(&draft.into_inner()).await?;
                if started {
                    self.log_stream.open(job.id());
                }
                if job.is_terminal() {
                    self.log_stream.close(job.id());
                    self.dispatch.wake(job.agent_app_id());
                }
                Ok(job)
            })
            .await
    }
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

#[async_trait]
impl Run<Prepare<CancelJob>> for JobUseCases {
    async fn run(&self, input: Authorized<CancelJob>) -> DomainResult<Prepared<CancelJob>> {
        let job = self.job_repo.find_by_id(&input.command().id).await?;
        Ok(input.prepared(Draft::new(job.cancel(clock::now())?)))
    }
}

#[async_trait]
impl Run<Persist<CancelJob>> for JobUseCases {
    async fn run(&self, input: Prepared<CancelJob>) -> DomainResult<Committed<CancelJob>> {
        input
            .commit(async |draft| {
                let job = self.job_repo.update(&draft.into_inner()).await?;
                self.dispatch.stop(&job);
                Ok(job)
            })
            .await
    }
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

#[async_trait]
impl Run<Prepare<DeleteJob>> for JobUseCases {
    async fn run(&self, input: Authorized<DeleteJob>) -> DomainResult<Prepared<DeleteJob>> {
        let job = self.job_repo.find_by_id(&input.command().id).await?;
        Ok(input.prepared(job))
    }
}

#[async_trait]
impl Run<Persist<DeleteJob>> for JobUseCases {
    async fn run(&self, input: Prepared<DeleteJob>) -> DomainResult<Committed<DeleteJob>> {
        input
            .commit(async |job| {
                self.job_repo.delete(&job).await?;
                if !job.is_terminal() {
                    self.dispatch.stop(&job);
                }
                Ok(Deleted::new(job))
            })
            .await
    }
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

#[async_trait]
impl Run<Prepare<ReapOrphanedJobs>> for JobUseCases {
    async fn run(
        &self,
        input: Authorized<ReapOrphanedJobs>,
    ) -> DomainResult<Prepared<ReapOrphanedJobs>> {
        service_only(input.caller())?;
        let now = clock::now();
        let stranded = self.job_repo.list_stranded(now - ORPHAN_GRACE).await?;
        let held = self.job_repo.pending_streams().await?;
        let open = self.dispatch.open_streams();
        let mut orphans = Vec::new();
        for job in stranded {
            if !job
                .agent_app_id()
                .is_some_and(|agent| open.iter().any(|s| &s.agent == agent))
            {
                orphans.push(Draft::new(job.orphan(now)?));
            }
        }
        let gone = held.into_iter().filter(|s| !open.contains(s)).collect();
        Ok(input.prepared((orphans, gone)))
    }
}

#[async_trait]
impl Run<Persist<ReapOrphanedJobs>> for JobUseCases {
    async fn run(
        &self,
        input: Prepared<ReapOrphanedJobs>,
    ) -> DomainResult<Committed<ReapOrphanedJobs>> {
        input
            .commit(async |(orphans, gone)| {
                let mut changed = self.orphan_all(orphans).await?;
                for stream in &gone {
                    changed += self.dispatch.release(stream).await?;
                }
                Ok(changed)
            })
            .await
    }
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

#[async_trait]
impl Run<Prepare<ReleaseAgentJobs>> for JobUseCases {
    async fn run(
        &self,
        input: Authorized<ReleaseAgentJobs>,
    ) -> DomainResult<Prepared<ReleaseAgentJobs>> {
        let agent = app_only(input.caller())?;
        let id = input.command().stream.clone();
        Ok(input.prepared(AgentStream { agent, id }))
    }
}

#[async_trait]
impl Run<Persist<ReleaseAgentJobs>> for JobUseCases {
    async fn run(
        &self,
        input: Prepared<ReleaseAgentJobs>,
    ) -> DomainResult<Committed<ReleaseAgentJobs>> {
        input
            .commit(async |stream| self.dispatch.release(&stream).await)
            .await
    }
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

#[async_trait]
impl Run<Prepare<ReconcileAgentJobs>> for JobUseCases {
    async fn run(
        &self,
        input: Authorized<ReconcileAgentJobs>,
    ) -> DomainResult<Prepared<ReconcileAgentJobs>> {
        let agent = app_only(input.caller())?;
        let listed = &input.command().running;
        let now = clock::now();
        let mut orphans = Vec::new();
        let mut kept = Vec::new();
        for job in self.job_repo.list_running_on(&agent).await? {
            if listed.contains(job.id()) {
                kept.push(job.id().clone());
            } else {
                orphans.push(Draft::new(job.orphan(now)?));
            }
        }
        let stale = listed
            .iter()
            .filter(|id| !kept.contains(id))
            .cloned()
            .collect();
        Ok(input.prepared((agent, orphans, kept, stale)))
    }
}

#[async_trait]
impl Run<Persist<ReconcileAgentJobs>> for JobUseCases {
    async fn run(
        &self,
        input: Prepared<ReconcileAgentJobs>,
    ) -> DomainResult<Committed<ReconcileAgentJobs>> {
        input
            .commit(async |(agent, orphans, kept, stale)| {
                let orphaned = self.orphan_all(orphans).await?;
                for id in &kept {
                    self.log_stream.open(id);
                }
                for id in &stale {
                    self.dispatch.cancel(&agent, id);
                }
                Ok(orphaned)
            })
            .await
    }
}
