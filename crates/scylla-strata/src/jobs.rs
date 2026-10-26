use crate::access::{AccessModel, CallerOf};
use crate::ports::JobDispatch;
use futures_util::stream::{self, StreamExt, TryStreamExt};
use scylla_auth::authz::{Visibility, VisibilityResolver};
use scylla_core::application::agent::dispatch::assemble_dispatch;
use scylla_core::application::agent::{AgentDispatch, AgentStream, DispatchPendingJobs};
use scylla_core::application::guards::{app_only, service_only};
use scylla_core::application::job::{
    AppendJobLogs, CancelJob, DeleteJob, GetJob, JobEvent, ListJobLogs, ListJobs,
    ListOrganizationJobs, ListPipelineJobs, ListProjectJobs, ORPHAN_GRACE, ReapOrphanedJobs,
    ReconcileAgentJobs, RecordJobStatus, ReleaseAgentJobs, TailJobLogs,
};
use scylla_core::application::pagination::PaginatedResult;
use scylla_core::application::{
    DispatchUseCases, JobLogLiveStream, JobLogRepository, JobLogStreamPort, JobRepository,
    SecretResolver,
};
use scylla_domain::domain::caller::CallerContext;
use scylla_domain::domain::clock;
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{AppId, JobId, PipelineId};
use scylla_domain::domain::job::{Job, JobLog, LogStream, NodeOutcome};
use scylla_domain::domain::permission::Permission;
use scylla_domain::domain::pipeline::NodeId;
use scylla_extension::Describe;
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

    #[usecase(write, access = cmd.access())]
    async fn record_job_status(&self, cmd: RecordJobStatus) -> DomainResult<Job> {
        let agent = app_only(self.ctx().caller())?;
        let job = self.job_store().find_by_id(&cmd.job_id).await?;
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
        let started = matches!(cmd.event, JobEvent::JobStarted);
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

    #[usecase(write, access = cmd.access())]
    async fn cancel_job(&self, cmd: CancelJob) -> DomainResult<Job> {
        let job = self.job_store().find_by_id(&cmd.id).await?;
        let job = job.cancel(clock::now())?;
        let job = self.job_store().update(&job).await?;
        self.job_stops().0.stop(&job);
        Ok(job)
    }

    #[usecase(write, access = cmd.access())]
    async fn delete_job(&self, cmd: DeleteJob) -> DomainResult<()> {
        let job = self.job_store().find_by_id(&cmd.id).await?;
        self.job_store().delete(&job).await?;
        if !job.is_terminal() {
            self.job_stops().0.stop(&job);
        }
        Ok(())
    }

    #[usecase(write, access = cmd.access())]
    #[allow(unused_variables)]
    async fn reap_orphaned_jobs(&self, cmd: ReapOrphanedJobs) -> DomainResult<u64> {
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
    #[usecase(write, access = cmd.access())]
    async fn dispatch_pending_jobs(&self, cmd: DispatchPendingJobs) -> DomainResult<Vec<Job>> {
        service_only(self.ctx().caller())?;
        let mut targets = self.job_agents().connected();
        if let Some(agents) = &cmd.agents {
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

    #[usecase(write, access = cmd.access())]
    async fn release_agent_jobs(&self, cmd: ReleaseAgentJobs) -> DomainResult<u64> {
        let agent = app_only(self.ctx().caller())?;
        let stream = AgentStream {
            agent,
            id: cmd.stream,
        };
        self.job_stops().0.release(&stream).await
    }

    #[usecase(write, access = cmd.access())]
    async fn reconcile_agent_jobs(&self, cmd: ReconcileAgentJobs) -> DomainResult<u64> {
        let agent = app_only(self.ctx().caller())?;
        let listed = &cmd.running;
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

    #[usecase(read, access = query.access())]
    async fn get_job(&self, query: GetJob) -> DomainResult<Job> {
        self.job_store().find_by_id(&query.id).await
    }

    #[usecase(read, access = query.access())]
    async fn list_jobs(&self, query: ListJobs) -> DomainResult<PaginatedResult<Job>> {
        self.job_store().list_all(query.pagination.as_ref()).await
    }

    #[usecase(read, access = query.access())]
    async fn list_pipeline_jobs(
        &self,
        query: ListPipelineJobs,
    ) -> DomainResult<PaginatedResult<Job>> {
        self.job_store()
            .list_by_pipeline(&query.pipeline_id, query.pagination.as_ref())
            .await
    }

    #[usecase(read, access = query.access())]
    async fn list_project_jobs(
        &self,
        query: ListProjectJobs,
    ) -> DomainResult<PaginatedResult<Job>> {
        self.job_store()
            .list_by_project(&query.project_id, query.pagination.as_ref())
            .await
    }

    #[usecase(read, access = query.access())]
    async fn list_organization_jobs(
        &self,
        query: ListOrganizationJobs,
    ) -> DomainResult<PaginatedResult<Job>> {
        self.job_store()
            .list_by_organization(
                &query.organization_id,
                query.agent_id.as_ref(),
                query.pagination.as_ref(),
            )
            .await
    }

    #[usecase(write, access = cmd.access())]
    async fn append_job_logs(&self, cmd: AppendJobLogs) -> DomainResult<Vec<JobLog>> {
        let agent = app_only(self.ctx().caller())?;
        if cmd.logs.iter().any(|log| log.job_id() != &cmd.job_id) {
            return Err(DomainError::validation("every line must belong to the job"));
        }
        self.job_store()
            .find_by_id(&cmd.job_id)
            .await?
            .ensure_live_on(&agent)?;
        self.job_log_store().create_many(&cmd.logs).await?;
        for log in &cmd.logs {
            self.job_tails().0.publish(log);
        }
        Ok(cmd.logs)
    }

    #[usecase(read, access = query.access())]
    async fn list_job_logs(&self, query: ListJobLogs) -> DomainResult<PaginatedResult<JobLog>> {
        let pagination = query.pagination.as_ref();
        if !self
            .job_node_readable(&query.job_id, query.node_id.as_ref())
            .await?
        {
            let params = query.pagination.unwrap_or_default();
            return Ok(PaginatedResult::new(Vec::new(), &params, 0));
        }
        match &query.node_id {
            Some(node_id) => {
                self.job_log_store()
                    .list_by_job_and_node(&query.job_id, node_id, pagination)
                    .await
            }
            None => {
                self.job_log_store()
                    .list_by_job(&query.job_id, pagination)
                    .await
            }
        }
    }

    #[usecase(read, access = query.access())]
    async fn tail_job_logs(&self, query: TailJobLogs) -> DomainResult<JobLogLiveStream> {
        let node_id = query.node_id.as_ref();
        let live = self.job_tails().0.subscribe(&query.job_id, node_id).await?;
        let historical = if self.job_node_readable(&query.job_id, node_id).await? {
            self.job_log_store()
                .list_all_by_job(&query.job_id, node_id)
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
