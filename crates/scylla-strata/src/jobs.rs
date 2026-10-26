use crate::access::{AccessModel, CallerOf};
use crate::ports::JobDispatch;
use futures_util::stream::{self, StreamExt, TryStreamExt};
use scylla_auth::authz::{Visibility, VisibilityResolver};
use scylla_core::application::agent::dispatch::assemble_dispatch;
use scylla_core::application::agent::{AgentDispatch, AgentStream};
use scylla_core::application::guards::{app_only, service_only};
use scylla_core::application::job::{JobEvent, ORPHAN_GRACE};
use scylla_core::application::pagination::{PaginatedResult, PaginationParams};
use scylla_core::application::{
    DispatchUseCases, JobLogLiveStream, JobLogRepository, JobLogStreamPort, JobRepository,
    SecretResolver,
};
use scylla_core::domain::access::Access;
use scylla_domain::domain::caller::CallerContext;
use scylla_domain::domain::clock;
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{AppId, JobId, OrganizationId, PipelineId, ProjectId, StreamId};
use scylla_domain::domain::job::{Job, JobLog, LogStream, NodeOutcome};
use scylla_domain::domain::permission::Permission;
use scylla_domain::domain::pipeline::NodeId;
use sqlx::types::chrono::{DateTime, Utc};
use std::collections::HashSet;
use std::sync::Arc;
use strata::{BuildCx, BuildError, Component, Resource};

#[derive(Clone)]
pub struct JobLogTails(pub Arc<dyn JobLogStreamPort>);

impl Resource for JobLogTails {
    const SECTION: &'static str = "job_log_tails";
    async fn build(_: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Err(BuildError::msg(
            "the live tail is built by the server: give it with `Builder::provide`",
        ))
    }
}

pub struct LiveTails(Arc<dyn JobLogStreamPort>);

impl Component for LiveTails {
    async fn build(cx: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Ok(LiveTails(cx.resource::<JobLogTails>().await?.0))
    }
}

/// The scopes in which an agent can run jobs. A dispatch pass reads them with no permission
/// check, so it writes no audit row.
pub struct AgentScopes(Arc<dyn VisibilityResolver>);

impl Component for AgentScopes {
    async fn build(cx: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Ok(AgentScopes(cx.resource::<AccessModel>().await?.visibility))
    }
}

pub struct JobStopper(Arc<DispatchUseCases>);

impl Component for JobStopper {
    async fn build(cx: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Ok(JobStopper(cx.resource::<JobDispatch>().await?.0))
    }
}

/// The access of a read of the logs of a job. A node also needs `readJob`: the node of a job is
/// read from the job.
fn job_logs_access(job_id: &JobId, node_id: Option<&NodeId>) -> Access {
    let logs = Permission::ReadJobLogs(job_id.clone());
    match node_id {
        Some(_) => Access::RequiresAll(vec![logs, Permission::ReadJob(job_id.clone())]),
        None => Access::Requires(logs),
    }
}

#[strata::module(section = "jobs")]
pub trait JobControl: crate::Access {
    #[component]
    type JobStore = Arc<dyn JobRepository>;

    #[component]
    type JobLogStore = Arc<dyn JobLogRepository>;

    #[component]
    type JobTails = LiveTails;

    #[component]
    type JobStops = JobStopper;

    #[component]
    type JobAgents = Arc<dyn AgentDispatch>;

    #[component]
    type JobSecrets = Arc<dyn SecretResolver>;

    #[component]
    type JobScopes = AgentScopes;

    /// Claims the oldest job that the agent can run and sends it on the stream. A job that
    /// does not assemble fails, and the agent gets the next job.
    async fn place_job(
        &self,
        stream: &AgentStream,
        visible: &Visibility,
    ) -> DomainResult<Option<Job>> {
        while let Some((job, project_id)) = self.job_store().claim_next(stream, visible).await? {
            match assemble_dispatch(self.job_secrets().as_ref(), &project_id, &job).await {
                Ok(dispatch) => {
                    if let Err(e) = self.job_agents().run(stream, dispatch) {
                        tracing::warn!(app_id = %stream.agent, job_id = %job.id(), error = %e, "could not send the job; it goes back to the pool");
                        self.job_stops().0.release(stream).await?;
                        return Ok(None);
                    }
                    return Ok(Some(job));
                }
                Err(e) => {
                    tracing::warn!(job_id = %job.id(), error = %e, "the job cannot be dispatched; it fails");
                    match self.job_store().update(&job.fail(clock::now())?).await {
                        Ok(_) | Err(DomainError::Stale(_)) => {}
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        Ok(None)
    }

    async fn orphan_jobs(&self, orphans: Vec<Job>) -> DomainResult<u64> {
        let mut written = 0;
        for job in orphans {
            match self.job_store().update(&job).await {
                Ok(job) => self.job_stops().0.stop(&job),
                Err(DomainError::Stale(_)) => continue,
                Err(e) => return Err(e),
            }
            written += 1;
        }
        Ok(written)
    }

    async fn job_node_readable(
        &self,
        job_id: &JobId,
        node_id: Option<&NodeId>,
    ) -> DomainResult<bool> {
        match node_id {
            Some(node_id) => Ok(self
                .job_store()
                .find_by_id(job_id)
                .await?
                .logs_readable_for(node_id)),
            None => Ok(true),
        }
    }

    /// One `writeJobStatus` check. The reads and the write go through the port, thus an agent
    /// needs no `readJob`. Only the agent that the job is placed on reports it, until the job
    /// ends. `at` is the time of the event on the agent, kept between the creation of the job and
    /// now. Each write of a job is versioned: a pass skips a job that changed since it read it
    /// (`Stale`).
    #[usecase(write, access = Access::Requires(Permission::WriteJobStatus(job_id.clone())))]
    async fn record_job_status(
        &self,
        job_id: JobId,
        event: JobEvent,
        at: DateTime<Utc>,
    ) -> DomainResult<Job> {
        let agent = app_only(self.ctx().caller())?;
        let job = self.job_store().find_by_id(&job_id).await?;
        job.ensure_live_on(&agent)?;
        let at = at.max(job.created_at()).min(clock::now());
        let job = match &event {
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
        let started = matches!(event, JobEvent::JobStarted);
        let job = self.job_store().update(&job).await?;
        if started {
            self.job_tails().0.open(job.id());
        }
        if job.is_terminal() {
            self.job_tails().0.close(job.id());
            self.job_stops().0.wake(job.agent_app_id());
        }
        Ok(job)
    }

    #[usecase(write, access = Access::Requires(Permission::UpdateJob(id.clone())))]
    async fn cancel_job(&self, id: JobId) -> DomainResult<Job> {
        let job = self.job_store().find_by_id(&id).await?;
        let job = job.cancel(clock::now())?;
        let job = self.job_store().update(&job).await?;
        self.job_stops().0.stop(&job);
        Ok(job)
    }

    #[usecase(write, access = Access::Requires(Permission::DeleteJob(id.clone())))]
    async fn delete_job(&self, id: JobId) -> DomainResult<()> {
        let job = self.job_store().find_by_id(&id).await?;
        self.job_store().delete(&job).await?;
        if !job.is_terminal() {
            self.job_stops().0.stop(&job);
        }
        Ok(())
    }

    /// One pass: a running job becomes orphaned when its agent is not connected and was not seen
    /// for `ORPHAN_GRACE`, or at once when its agent row is gone. The jobs placed on a stream that
    /// is not open go back to the pool. The open streams are read after the jobs: a stream opens
    /// before it holds a job, thus a stream that the pass finds closed is gone for good. The
    /// return is the number of jobs changed.
    #[usecase(write, access = Access::Authenticated)]
    async fn reap_orphaned_jobs(&self) -> DomainResult<u64> {
        service_only(self.ctx().caller())?;
        let now = clock::now();
        let stranded = self.job_store().list_stranded(now - ORPHAN_GRACE).await?;
        let held = self.job_store().pending_streams().await?;
        let open = self.job_stops().0.open_streams();
        let mut orphans = Vec::new();
        for job in stranded {
            if !job
                .agent_app_id()
                .is_some_and(|agent| open.iter().any(|s| &s.agent == agent))
            {
                orphans.push(job.orphan(now)?);
            }
        }
        let gone: Vec<AgentStream> = held.into_iter().filter(|s| !open.contains(s)).collect();
        let mut changed = self.orphan_jobs(orphans).await?;
        for stream in &gone {
            changed += self.job_stops().0.release(stream).await?;
        }
        Ok(changed)
    }

    /// One pass: each targeted agent that is connected, idle and holds `executeJob` in a scope
    /// gets the oldest pending job that it can run. `None` targets every connected agent.
    #[usecase(write, access = Access::Authenticated)]
    async fn dispatch_pending_jobs(&self, agents: Option<Vec<AppId>>) -> DomainResult<Vec<Job>> {
        service_only(self.ctx().caller())?;
        let mut targets = self.job_agents().connected();
        if let Some(agents) = &agents {
            targets.retain(|stream| agents.contains(&stream.agent));
        }
        let agents: Vec<AppId> = targets.iter().map(|stream| stream.agent.clone()).collect();
        let busy: HashSet<AppId> = self
            .job_store()
            .active_jobs(&agents)
            .await?
            .into_iter()
            .map(|(agent, _)| agent)
            .collect();
        let execute = Permission::ExecuteJob(PipelineId::new("_"));
        let mut staged = Vec::new();
        for stream in targets.into_iter().filter(|s| !busy.contains(&s.agent)) {
            let visible = self
                .job_scopes()
                .0
                .visible_scopes(&CallerContext::App(stream.agent.clone()), execute.key())
                .await?;
            if !visible.is_empty() {
                staged.push((stream, visible));
            }
        }
        let mut placed = Vec::new();
        for (stream, visible) in staged {
            placed.extend(self.place_job(&stream, &visible).await?);
        }
        Ok(placed)
    }

    /// Sent by the agent stream as the agent when the stream ends: the jobs placed on the stream
    /// that have not started go back to the pool. The return is their count.
    #[usecase(write, access = Access::Authenticated)]
    async fn release_agent_jobs(&self, stream: StreamId) -> DomainResult<u64> {
        let agent = app_only(self.ctx().caller())?;
        let stream = AgentStream { agent, id: stream };
        self.job_stops().0.release(&stream).await
    }

    /// Sent by the agent stream after each hello, as the agent. `running` is the list of jobs that
    /// the agent runs: a running job of the agent that the list omits becomes orphaned, a listed
    /// running job keeps its live tail across a restart of the server, and a listed job that does
    /// not run on the agent gets a cancel. The return is the number of jobs changed.
    #[usecase(write, access = Access::Authenticated)]
    async fn reconcile_agent_jobs(&self, running: Vec<JobId>) -> DomainResult<u64> {
        let agent = app_only(self.ctx().caller())?;
        let listed = &running;
        let now = clock::now();
        let mut orphans = Vec::new();
        let mut kept = Vec::new();
        for job in self.job_store().list_running_on(&agent).await? {
            if listed.contains(job.id()) {
                kept.push(job.id().clone());
            } else {
                orphans.push(job.orphan(now)?);
            }
        }
        let stale: Vec<JobId> = listed
            .iter()
            .filter(|id| !kept.contains(id))
            .cloned()
            .collect();
        let orphaned = self.orphan_jobs(orphans).await?;
        for id in &kept {
            self.job_tails().0.open(id);
        }
        for id in &stale {
            self.job_stops().0.cancel(&agent, id);
        }
        Ok(orphaned)
    }

    #[usecase(read, access = Access::Requires(Permission::ReadJob(id.clone())))]
    async fn get_job(&self, id: JobId) -> DomainResult<Job> {
        self.job_store().find_by_id(&id).await
    }

    #[usecase(read, access = Access::Requires(Permission::ListJobs))]
    async fn list_jobs(
        &self,
        pagination: Option<PaginationParams>,
    ) -> DomainResult<PaginatedResult<Job>> {
        self.job_store().list_all(pagination.as_ref()).await
    }

    #[usecase(read, access = Access::Requires(Permission::ListJobsByPipeline(pipeline_id.clone())))]
    async fn list_pipeline_jobs(
        &self,
        pipeline_id: PipelineId,
        pagination: Option<PaginationParams>,
    ) -> DomainResult<PaginatedResult<Job>> {
        self.job_store()
            .list_by_pipeline(&pipeline_id, pagination.as_ref())
            .await
    }

    #[usecase(read, access = Access::Requires(Permission::ListJobsByProject(project_id.clone())))]
    async fn list_project_jobs(
        &self,
        project_id: ProjectId,
        pagination: Option<PaginationParams>,
    ) -> DomainResult<PaginatedResult<Job>> {
        self.job_store()
            .list_by_project(&project_id, pagination.as_ref())
            .await
    }

    /// With `agent_id`, only the jobs placed on that agent.
    #[usecase(read, access = Access::Requires(Permission::ListJobsByOrganization(organization_id.clone())))]
    async fn list_organization_jobs(
        &self,
        organization_id: OrganizationId,
        agent_id: Option<AppId>,
        pagination: Option<PaginationParams>,
    ) -> DomainResult<PaginatedResult<Job>> {
        self.job_store()
            .list_by_organization(&organization_id, agent_id.as_ref(), pagination.as_ref())
            .await
    }

    /// Consecutive lines of one job from its agent, in the order received: one check and one
    /// insert for the batch. Only the agent that the job is placed on appends, until the job ends.
    #[usecase(write, access = Access::Requires(Permission::AppendJobLog(job_id.clone())))]
    async fn append_job_logs(&self, job_id: JobId, logs: Vec<JobLog>) -> DomainResult<Vec<JobLog>> {
        let agent = app_only(self.ctx().caller())?;
        if logs.iter().any(|log| log.job_id() != &job_id) {
            return Err(DomainError::validation("every line must belong to the job"));
        }
        self.job_store()
            .find_by_id(&job_id)
            .await?
            .ensure_live_on(&agent)?;
        self.job_log_store().create_many(&logs).await?;
        for log in &logs {
            self.job_tails().0.publish(log);
        }
        Ok(logs)
    }

    #[usecase(read, access = job_logs_access(&job_id, node_id.as_ref()))]
    async fn list_job_logs(
        &self,
        job_id: JobId,
        node_id: Option<NodeId>,
        pagination: Option<PaginationParams>,
    ) -> DomainResult<PaginatedResult<JobLog>> {
        if !self.job_node_readable(&job_id, node_id.as_ref()).await? {
            let params = pagination.unwrap_or_default();
            return Ok(PaginatedResult::new(Vec::new(), &params, 0));
        }
        let pagination = pagination.as_ref();
        match &node_id {
            Some(node_id) => {
                self.job_log_store()
                    .list_by_job_and_node(&job_id, node_id, pagination)
                    .await
            }
            None => self.job_log_store().list_by_job(&job_id, pagination).await,
        }
    }

    /// The stored lines, then the live ones. A live line already in the snapshot is dropped. A
    /// node that has not started replays nothing.
    #[usecase(read, access = job_logs_access(&job_id, node_id.as_ref()))]
    async fn tail_job_logs(
        &self,
        job_id: JobId,
        node_id: Option<NodeId>,
    ) -> DomainResult<JobLogLiveStream> {
        let node_id = node_id.as_ref();
        let live = self.job_tails().0.subscribe(&job_id, node_id).await?;
        let historical = if self.job_node_readable(&job_id, node_id).await? {
            self.job_log_store()
                .list_all_by_job(&job_id, node_id)
                .await?
        } else {
            Vec::new()
        };
        let cutoff = historical.last().map(JobLog::timestamp);
        let seen: HashSet<(DateTime<Utc>, NodeId, LogStream, String)> = historical
            .iter()
            .map(|l| {
                (
                    l.timestamp(),
                    l.node_id().clone(),
                    *l.stream(),
                    l.line().to_string(),
                )
            })
            .collect();
        let historical_stream = stream::iter(historical.into_iter().map(Ok));
        let filtered_live = live.try_filter(move |log| {
            let strictly_newer = cutoff.is_none_or(|c| log.timestamp() > c);
            let keep = strictly_newer
                || !seen.contains(&(
                    log.timestamp(),
                    log.node_id().clone(),
                    *log.stream(),
                    log.line().to_string(),
                ));
            std::future::ready(keep)
        });
        let tail: JobLogLiveStream = Box::pin(historical_stream.chain(filtered_live));
        Ok(tail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_of_the_logs_of_a_node_also_needs_read_job() {
        let job = JobId::new("j1");
        let node = NodeId::new("n1").unwrap();

        assert_eq!(
            job_logs_access(&job, Some(&node)).to_string(),
            "readJobLogs+readJob"
        );
        assert_eq!(job_logs_access(&job, None).to_string(), "readJobLogs");
    }
}
