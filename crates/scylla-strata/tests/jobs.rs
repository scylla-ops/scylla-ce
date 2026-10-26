use async_trait::async_trait;
use scylla_auth::authz::{PermissionService, Visibility, VisibilityResolver};
use scylla_core::application::agent::{AgentDispatch, AgentStream, DispatchNode};
use scylla_core::application::job::{JobEvent, JobScope};
use scylla_core::application::pagination::{PaginatedResult, PaginationParams};
use scylla_core::application::{
    JobLogLiveStream, JobLogRepository, JobLogStreamPort, JobRepository, SecretResolver,
};
use scylla_core::infrastructure::InMemoryJobLogStream;
use scylla_core::test_support::authz::{DenyingPermissionService, RecordingPermissionService};
use scylla_core::test_support::job_logs::{JobLogBuilder, job_log};
use scylla_core::test_support::jobs::{JobBuilder, job, stored};
use scylla_core::test_support::pipelines::{PipelineBuilder, node};
use scylla_core::test_support::stubs::{
    EchoResolver, ScopesByAgent, StubJobs, StubRegistry, StubTails, dispatcher,
};
use scylla_domain::domain::caller::{CallerContext, ServiceIdentity};
use scylla_domain::domain::clock;
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{
    AppId, JobId, OrganizationId, PipelineId, ProjectId, StreamId, UserId,
};
use scylla_domain::domain::job::{Job, JobLog, JobStatus, NodeState};
use scylla_domain::domain::permission::Permission;
use scylla_domain::domain::pipeline::{NodeId, Pipeline, PipelineNode};
use scylla_proto::common::v1::JobId as PbJobId;
use scylla_proto::job::v1::{GetJobRequest, TailJobLogsRequest};
use scylla_strata::grpc::{JobServiceClient, JobServiceServer};
use scylla_strata::{
    AccessModel, CedarPermissions, JobControlRpc, JobDispatch, JobLogTails, ScyllaCtx,
};
use sqlx::types::chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;
use strata::{Config, Contextual, Edition};
use strata_tonic::Grpc;
use tokio_stream::StreamExt;
use tonic::Code;
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Channel, Server};

#[strata::edition(modules(scylla_strata::Access, scylla_strata::JobControl))]
pub struct TestBase;

#[strata::edition(extends = TestBase, overrides(scylla_strata::Access))]
pub struct TestCe;

#[strata::overrides]
impl scylla_strata::Access for TestCe {
    type Permissions = CedarPermissions;
}

struct SeeAll;

#[async_trait]
impl VisibilityResolver for SeeAll {
    async fn visible_scopes(&self, _: &CallerContext, _: &str) -> DomainResult<Visibility> {
        Ok(Visibility::All)
    }
}

enum Race {
    ReportLandsFirst(JobId),
    AgentComesBack {
        gone: AgentStream,
        back: AgentStream,
    },
}

struct Racing {
    inner: Arc<StubJobs>,
    race: Mutex<Option<Race>>,
}

#[async_trait]
impl JobRepository for Racing {
    async fn create(&self, job: &Job) -> DomainResult<Job> {
        self.inner.create(job).await
    }
    async fn find_by_id(&self, id: &JobId) -> DomainResult<Job> {
        self.inner.find_by_id(id).await
    }
    async fn update(&self, job: &Job) -> DomainResult<Job> {
        self.inner.update(job).await
    }
    async fn delete(&self, job: &Job) -> DomainResult<()> {
        self.inner.delete(job).await
    }
    async fn claim_next(
        &self,
        stream: &AgentStream,
        visible: &Visibility,
    ) -> DomainResult<Option<(Job, ProjectId)>> {
        self.inner.claim_next(stream, visible).await
    }
    async fn release(&self, stream: &AgentStream) -> DomainResult<u64> {
        self.inner.release(stream).await
    }
    async fn pending_streams(&self) -> DomainResult<Vec<AgentStream>> {
        let held = self.inner.pending_streams().await?;
        let race = self.race.lock().unwrap().take();
        match race {
            Some(Race::ReportLandsFirst(id)) => self.inner.touch(&id),
            Some(Race::AgentComesBack { gone, back }) => {
                self.inner.release(&gone).await?;
                self.inner.claim_next(&back, &Visibility::All).await?;
            }
            None => {}
        }
        Ok(held)
    }
    async fn active_jobs(&self, agents: &[AppId]) -> DomainResult<Vec<(AppId, u32)>> {
        self.inner.active_jobs(agents).await
    }
    async fn list_live(&self, scope: JobScope<'_>) -> DomainResult<Vec<Job>> {
        self.inner.list_live(scope).await
    }
    async fn list_running_on(&self, agent: &AppId) -> DomainResult<Vec<Job>> {
        self.inner.list_running_on(agent).await
    }
    async fn list_stranded(&self, seen_before: DateTime<Utc>) -> DomainResult<Vec<Job>> {
        self.inner.list_stranded(seen_before).await
    }
    async fn list_all(
        &self,
        page: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Job>> {
        self.inner.list_all(page).await
    }
    async fn list_by_pipeline(
        &self,
        id: &PipelineId,
        page: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Job>> {
        self.inner.list_by_pipeline(id, page).await
    }
    async fn list_by_project(
        &self,
        id: &ProjectId,
        page: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Job>> {
        self.inner.list_by_project(id, page).await
    }
    async fn list_by_organization(
        &self,
        id: &OrganizationId,
        agent: Option<&AppId>,
        page: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Job>> {
        self.inner.list_by_organization(id, agent, page).await
    }
}

#[derive(Default)]
struct StubLogs {
    rows: Mutex<Vec<JobLog>>,
    reads: Mutex<Vec<&'static str>>,
}

fn page(items: Vec<JobLog>) -> PaginatedResult<JobLog> {
    let total = items.len() as u64;
    PaginatedResult::new(items, &PaginationParams::default(), total)
}

#[async_trait]
impl JobLogRepository for StubLogs {
    async fn create_many(&self, logs: &[JobLog]) -> DomainResult<()> {
        self.reads.lock().unwrap().push("insert");
        self.rows.lock().unwrap().extend_from_slice(logs);
        Ok(())
    }
    async fn list_by_job(
        &self,
        _: &JobId,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<JobLog>> {
        self.reads.lock().unwrap().push("job");
        Ok(page(self.rows.lock().unwrap().clone()))
    }
    async fn list_by_job_and_node(
        &self,
        _: &JobId,
        node_id: &NodeId,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<JobLog>> {
        self.reads.lock().unwrap().push("node");
        let rows = self.rows.lock().unwrap();
        Ok(page(
            rows.iter()
                .filter(|l| l.node_id() == node_id)
                .cloned()
                .collect(),
        ))
    }
    async fn list_all_by_job(&self, _: &JobId, _: Option<&NodeId>) -> DomainResult<Vec<JobLog>> {
        Ok(self.rows.lock().unwrap().clone())
    }
}

#[derive(Default)]
struct StubLive {
    lines: Mutex<Vec<JobLog>>,
    subscribes: Mutex<usize>,
    published: Mutex<Vec<JobLog>>,
}

#[async_trait]
impl JobLogStreamPort for StubLive {
    fn open(&self, _: &JobId) {}
    fn publish(&self, log: &JobLog) {
        self.published.lock().unwrap().push(log.clone());
    }
    fn close(&self, _: &JobId) {}
    async fn subscribe(&self, _: &JobId, _: Option<&NodeId>) -> DomainResult<JobLogLiveStream> {
        *self.subscribes.lock().unwrap() += 1;
        let lines = self.lines.lock().unwrap().clone();
        Ok(Box::pin(futures_util::stream::iter(
            lines.into_iter().map(Ok),
        )))
    }
}

fn ctx(caller: CallerContext) -> ScyllaCtx {
    ScyllaCtx {
        caller,
        ..ScyllaCtx::default()
    }
}

fn alice() -> ScyllaCtx {
    ctx(CallerContext::User(UserId::new("alice")))
}

fn agent_id() -> AppId {
    AppId::new("agent")
}

fn agent() -> ScyllaCtx {
    ctx(CallerContext::App(agent_id()))
}

fn reaper() -> ScyllaCtx {
    ctx(CallerContext::Service(ServiceIdentity::job_reaper()))
}

async fn build(
    permissions: Arc<dyn PermissionService>,
    jobs: Arc<dyn JobRepository>,
    dispatch_jobs: Arc<StubJobs>,
    registry: Arc<StubRegistry>,
    logs: Arc<dyn JobLogRepository>,
    tails: Arc<dyn JobLogStreamPort>,
    stub_tails: Arc<StubTails>,
) -> TestCe {
    TestCe::builder(&Config::new())
        .provide(AccessModel {
            permissions,
            visibility: Arc::new(SeeAll),
        })
        .provide(JobDispatch(dispatcher(
            registry.clone(),
            dispatch_jobs,
            stub_tails,
        )))
        .provide_arc::<dyn AgentDispatch>(registry)
        .provide_arc::<dyn SecretResolver>(Arc::new(EchoResolver))
        .provide_arc::<dyn JobRepository>(jobs)
        .provide_arc::<dyn JobLogRepository>(logs)
        .provide(JobLogTails(tails))
        .build()
        .await
        .unwrap()
}

struct Lab {
    app: TestCe,
    jobs: Arc<StubJobs>,
    registry: Arc<StubRegistry>,
    tails: Arc<StubTails>,
    stream: AgentStream,
}

async fn lab_racing(
    permissions: Arc<dyn PermissionService>,
    race: Option<Race>,
    jobs: Arc<StubJobs>,
) -> Lab {
    let registry = Arc::new(StubRegistry::default());
    let stream = registry.connect(&agent_id());
    let tails = Arc::new(StubTails::default());
    let racing = Arc::new(Racing {
        inner: jobs.clone(),
        race: Mutex::new(race),
    });
    let app = build(
        permissions,
        racing,
        jobs.clone(),
        registry.clone(),
        Arc::new(StubLogs::default()),
        tails.clone(),
        tails.clone(),
    )
    .await;
    Lab {
        app,
        jobs,
        registry,
        tails,
        stream,
    }
}

async fn lab(permissions: Arc<dyn PermissionService>) -> Lab {
    lab_racing(permissions, None, Arc::new(StubJobs::default())).await
}

impl Lab {
    fn insert(&self, job: &Job) -> Job {
        self.jobs.insert(job)
    }

    async fn record(&self, job: &Job, event: JobEvent) -> DomainResult<Job> {
        self.record_at(job, event, clock::now()).await
    }

    async fn record_at(&self, job: &Job, event: JobEvent, at: DateTime<Utc>) -> DomainResult<Job> {
        JobControlRpc::record_job_status(&self.app.scoped(&agent()), job.id().clone(), event, at)
            .await
    }

    async fn cancel(&self, id: &JobId) -> DomainResult<Job> {
        JobControlRpc::cancel_job(&self.app.scoped(&alice()), id.clone()).await
    }

    async fn delete(&self, id: &JobId) -> DomainResult<()> {
        JobControlRpc::delete_job(&self.app.scoped(&alice()), id.clone()).await
    }

    async fn reap(&self) -> DomainResult<u64> {
        JobControlRpc::reap_orphaned_jobs(&self.app.scoped(&reaper())).await
    }

    async fn release(&self, stream: &AgentStream) -> DomainResult<u64> {
        JobControlRpc::release_agent_jobs(&self.app.scoped(&agent()), stream.id.clone()).await
    }

    async fn reconcile(&self, running: &[&Job]) -> DomainResult<u64> {
        let running = running.iter().map(|job| job.id().clone()).collect();
        JobControlRpc::reconcile_agent_jobs(&self.app.scoped(&agent()), running).await
    }
}

fn two_nodes() -> Pipeline {
    PipelineBuilder::for_project_id(ProjectId::new("p"))
        .nodes(vec![node("a", &[]), node("b", &["a"])])
        .build()
}

fn placed(pipeline: &Pipeline) -> Job {
    JobBuilder::new(pipeline).agent(agent_id()).build()
}

fn busy(pipeline: &Pipeline) -> Job {
    placed(pipeline)
        .start(clock::now())
        .unwrap()
        .apply_node_started(&NodeId::new("a").unwrap(), clock::now())
        .unwrap()
}

fn another_stream(agent: &AppId) -> AgentStream {
    AgentStream {
        agent: agent.clone(),
        id: StreamId::generate(),
    }
}

#[tokio::test]
async fn a_status_event_of_the_agent_checks_write_job_status_and_persists_the_transition() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let seeded = lab.insert(&placed(&two_nodes()));

    let job = lab.record(&seeded, JobEvent::JobStarted).await.unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::WriteJobStatus(seeded.id().clone())]
    );
    assert_eq!(job.status(), JobStatus::Running);
    assert_eq!(lab.jobs.row(seeded.id()).status(), JobStatus::Running);
    assert_eq!(lab.tails.opened(), [seeded.id().clone()]);
}

#[tokio::test]
async fn a_denied_status_event_writes_nothing() {
    let lab = lab(Arc::new(DenyingPermissionService::new())).await;
    let seeded = lab.insert(&placed(&two_nodes()));

    let err = lab.record(&seeded, JobEvent::JobStarted).await.unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(lab.jobs.row(seeded.id()).status(), JobStatus::Pending);
}

#[tokio::test]
async fn only_the_agent_of_a_live_job_reports_it() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let pl = two_nodes();
    let elsewhere = lab.insert(&JobBuilder::new(&pl).agent(AppId::new("other")).build());
    let unplaced = lab.insert(&job(&pl));
    let ended = lab.insert(
        &JobBuilder::new(&pl)
            .terminated(JobStatus::Orphaned)
            .agent(agent_id())
            .build(),
    );

    for job in [&elsewhere, &unplaced, &ended] {
        let err = lab.record(job, JobEvent::JobStarted).await.unwrap_err();
        assert!(matches!(err, DomainError::BusinessRule(_)), "{err}");
    }
    let err = JobControlRpc::record_job_status(
        &lab.app.scoped(&alice()),
        elsewhere.id().clone(),
        JobEvent::JobStarted,
        clock::now(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(lab.jobs.row(ended.id()).status(), JobStatus::Orphaned);
}

#[tokio::test]
async fn the_time_of_an_event_stays_between_the_creation_and_now() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let seeded = lab.insert(&placed(&two_nodes()));

    let early = lab
        .record_at(
            &seeded,
            JobEvent::JobStarted,
            seeded.created_at() - StdDuration::from_secs(3600),
        )
        .await
        .unwrap();
    let late = lab
        .record_at(
            &early,
            JobEvent::JobFailed {
                error: "boom".into(),
            },
            clock::now() + StdDuration::from_secs(3600),
        )
        .await
        .unwrap();

    assert_eq!(early.started_at(), Some(seeded.created_at()));
    assert!(late.finished_at().unwrap() <= clock::now());
}

#[tokio::test]
async fn an_ended_job_closes_its_tail_and_frees_its_agent() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let seeded = lab.insert(&busy(&two_nodes()));

    let job = lab.record(&seeded, JobEvent::JobCompleted).await.unwrap();

    assert_eq!(job.status(), JobStatus::Completed);
    assert_eq!(
        job.find_execution(&NodeId::new("a").unwrap())
            .unwrap()
            .state(),
        NodeState::Cancelled
    );
    assert_eq!(lab.tails.closed(), [seeded.id().clone()]);
    assert_eq!(lab.registry.wakes(), [Some(agent_id())]);
}

#[tokio::test]
async fn a_cancel_ends_a_live_job_and_stops_it_on_its_agent() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let pl = two_nodes();
    let running = lab.insert(&busy(&pl));
    let pending = lab.insert(&job(&pl));

    let cancelled = lab.cancel(running.id()).await.unwrap();
    let never_placed = lab.cancel(pending.id()).await.unwrap();

    assert_eq!(cancelled.status(), JobStatus::Cancelled);
    assert_eq!(never_placed.status(), JobStatus::Cancelled);
    for node in cancelled.node_executions() {
        assert_eq!(node.state(), NodeState::Cancelled);
    }
    assert_eq!(
        lab.registry.cancelled(),
        [(agent_id(), running.id().clone())]
    );
    assert_eq!(lab.registry.wakes(), [Some(agent_id())]);
    assert_eq!(lab.tails.closed().len(), 2);
    assert_eq!(
        permissions.permissions(),
        vec![
            Permission::UpdateJob(running.id().clone()),
            Permission::UpdateJob(pending.id().clone()),
        ]
    );
}

#[tokio::test]
async fn a_cancel_of_an_ended_job_fails_its_precondition() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let ended = lab.insert(
        &JobBuilder::new(&two_nodes())
            .terminated(JobStatus::Completed)
            .build(),
    );

    let err = lab.cancel(ended.id()).await.unwrap_err();

    assert!(matches!(err, DomainError::BusinessRule(_)));
    assert!(lab.registry.cancelled().is_empty());
}

#[tokio::test]
async fn a_delete_removes_the_row_and_stops_a_live_job() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let pl = two_nodes();
    let running = lab.insert(&busy(&pl));
    let ended = lab.insert(
        &JobBuilder::new(&pl)
            .terminated(JobStatus::Failed)
            .agent(agent_id())
            .build(),
    );

    lab.delete(running.id()).await.unwrap();
    lab.delete(ended.id()).await.unwrap();

    assert!(lab.jobs.rows().is_empty());
    assert_eq!(
        lab.registry.cancelled(),
        [(agent_id(), running.id().clone())]
    );
    assert_eq!(
        permissions.permissions()[0],
        Permission::DeleteJob(running.id().clone())
    );
}

#[tokio::test]
async fn a_delete_of_a_missing_job_is_not_found_and_stops_nothing() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;

    let err = lab.delete(&JobId::new("gone")).await.unwrap_err();

    assert!(matches!(err, DomainError::NotFound(_)));
    assert!(lab.registry.cancelled().is_empty());
}

#[tokio::test]
async fn each_read_checks_its_own_permission() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let seeded = lab.insert(&job(&two_nodes()));
    let caller = alice();
    let alice = lab.app.scoped(&caller);

    let got = JobControlRpc::get_job(&alice, seeded.id().clone())
        .await
        .unwrap();
    JobControlRpc::list_jobs(&alice, None).await.unwrap();
    JobControlRpc::list_pipeline_jobs(&alice, PipelineId::new("pipe"), None)
        .await
        .unwrap();
    JobControlRpc::list_project_jobs(&alice, ProjectId::new("p"), None)
        .await
        .unwrap();
    JobControlRpc::list_organization_jobs(&alice, OrganizationId::new("acme"), None, None)
        .await
        .unwrap();

    assert_eq!(got.id(), seeded.id());
    assert_eq!(
        permissions.permissions(),
        vec![
            Permission::ReadJob(seeded.id().clone()),
            Permission::ListJobs,
            Permission::ListJobsByPipeline(PipelineId::new("pipe")),
            Permission::ListJobsByProject(ProjectId::new("p")),
            Permission::ListJobsByOrganization(OrganizationId::new("acme")),
        ]
    );
}

#[tokio::test]
async fn an_organization_list_with_an_agent_keeps_its_permission_and_lists_only_that_agent() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let pipeline = two_nodes();
    let on_agent = lab.insert(&placed(&pipeline));
    let ended = lab.insert(
        &JobBuilder::new(&pipeline)
            .terminated(JobStatus::Completed)
            .agent(agent_id())
            .build(),
    );
    lab.insert(
        &JobBuilder::new(&pipeline)
            .agent(AppId::new("other"))
            .build(),
    );
    lab.insert(&job(&pipeline));

    let page = JobControlRpc::list_organization_jobs(
        &lab.app.scoped(&alice()),
        OrganizationId::new("acme"),
        Some(agent_id()),
        None,
    )
    .await
    .unwrap();

    let ids: Vec<&JobId> = page.items().iter().map(Job::id).collect();
    assert_eq!(ids, [on_agent.id(), ended.id()]);
    assert_eq!(
        permissions.permissions(),
        [Permission::ListJobsByOrganization(OrganizationId::new(
            "acme"
        ))]
    );
}

#[tokio::test]
async fn a_reap_pass_ends_the_running_jobs_of_gone_agents_and_frees_their_streams() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let pl = two_nodes();
    let gone = AppId::new("gone");
    let lost = lab.insert(&stored(&busy(&pl), Some(&gone)));
    let unstarted = lab.jobs.place(&job(&pl), &another_stream(&gone));
    let kept = lab.insert(&busy(&pl));

    let changed = lab.reap().await.unwrap();

    assert_eq!(changed, 2);
    let orphaned = lab.jobs.row(lost.id());
    assert_eq!(orphaned.status(), JobStatus::Orphaned);
    for node in orphaned.node_executions() {
        assert_eq!(node.state(), NodeState::Cancelled);
    }
    assert!(lab.jobs.row(unstarted.id()).agent_app_id().is_none());
    assert_eq!(lab.jobs.row(kept.id()).status(), JobStatus::Running);
    assert_eq!(lab.tails.closed(), [lost.id().clone()]);
    assert!(lab.registry.wakes().contains(&None));
    assert!(permissions.permissions().is_empty());
}

#[tokio::test]
async fn a_reap_returns_the_number_of_jobs_it_changed() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    lab.insert(
        &JobBuilder::new(&two_nodes())
            .running(true)
            .agent(AppId::new("agent-gone"))
            .build(),
    );

    assert_eq!(lab.reap().await.unwrap(), 1);
    assert_eq!(lab.reap().await.unwrap(), 0);
    assert_eq!(lab.jobs.rows()[0].status(), JobStatus::Orphaned);
}

#[tokio::test]
async fn a_reap_pass_frees_a_stream_that_is_gone_while_its_agent_is_back() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let pl = two_nodes();
    let earlier = lab.jobs.place(&job(&pl), &another_stream(&agent_id()));
    let current = lab.jobs.place(&job(&pl), &lab.stream);

    assert_eq!(lab.reap().await.unwrap(), 1);

    assert!(lab.jobs.row(earlier.id()).agent_app_id().is_none());
    assert_eq!(lab.jobs.row(current.id()).agent_app_id(), Some(&agent_id()));
}

#[tokio::test]
async fn a_reap_pass_skips_a_job_that_changed_since_it_read_it() {
    let jobs = Arc::new(StubJobs::default());
    let lost = jobs.insert(&stored(&busy(&two_nodes()), Some(&AppId::new("gone"))));
    let lab = lab_racing(
        Arc::new(RecordingPermissionService::new()),
        Some(Race::ReportLandsFirst(lost.id().clone())),
        jobs,
    )
    .await;

    let changed = lab.reap().await.unwrap();

    assert_eq!(changed, 0);
    assert_eq!(lab.jobs.row(lost.id()).status(), JobStatus::Running);
    assert!(lab.tails.closed().is_empty());
}

#[tokio::test]
async fn a_reap_pass_leaves_the_job_that_a_new_stream_took_since_it_read() {
    let jobs = Arc::new(StubJobs::default());
    let returning = AppId::new("returning");
    let gone = another_stream(&returning);
    let waiting = jobs.place(&job(&two_nodes()), &gone);
    let back = another_stream(&returning);
    let lab = lab_racing(
        Arc::new(RecordingPermissionService::new()),
        Some(Race::AgentComesBack {
            gone: gone.clone(),
            back: back.clone(),
        }),
        jobs,
    )
    .await;

    let changed = lab.reap().await.unwrap();

    assert_eq!(changed, 0);
    assert_eq!(lab.jobs.row(waiting.id()).agent_app_id(), Some(&returning));
    assert_eq!(lab.jobs.stream_of(waiting.id()), Some(back.id));
}

#[tokio::test]
async fn a_reap_pass_refuses_a_caller_that_is_not_a_service() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    lab.jobs
        .place(&job(&two_nodes()), &another_stream(&agent_id()));

    let err = JobControlRpc::reap_orphaned_jobs(&lab.app.scoped(&alice()))
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.jobs.released().is_empty());
}

#[tokio::test]
async fn the_reaper_worker_passes_as_the_reaper_service() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    lab.insert(
        &JobBuilder::new(&two_nodes())
            .running(true)
            .agent(AppId::new("agent-gone"))
            .build(),
    );

    assert_eq!(scylla_strata::reap_jobs(&lab.app).await, 1);
    assert_eq!(scylla_strata::reap_jobs(&lab.app).await, 0);
    assert_eq!(lab.jobs.rows()[0].status(), JobStatus::Orphaned);
}

async fn dispatch_lab(jobs: Vec<Job>) -> (TestCe, Arc<StubJobs>, Arc<StubRegistry>) {
    let jobs = Arc::new(StubJobs::with(jobs));
    let registry = Arc::new(StubRegistry::accepting());
    let tails = Arc::new(StubTails::default());
    let app = build(
        Arc::new(RecordingPermissionService::new()),
        jobs.clone(),
        jobs.clone(),
        registry.clone(),
        Arc::new(StubLogs::default()),
        tails.clone(),
        tails,
    )
    .await;
    (app, jobs, registry)
}

#[tokio::test]
async fn the_dispatch_worker_places_one_job_on_each_idle_agent_it_targets() {
    let pl = two_nodes();
    let (app, jobs, registry) = dispatch_lab(vec![job(&pl), job(&pl)]).await;
    let agent = AppId::new("agent-1");
    registry.connect(&agent);

    let other = Some(vec![AppId::new("agent-2")]);
    assert_eq!(scylla_strata::dispatch_jobs(&app, other).await, 0);
    assert_eq!(scylla_strata::dispatch_jobs(&app, None).await, 1);
    assert_eq!(
        scylla_strata::dispatch_jobs(&app, None).await,
        0,
        "one job per agent"
    );
    assert_eq!(registry.dispatched_to(), std::slice::from_ref(&agent));
    let placed: Vec<Job> = jobs
        .rows()
        .into_iter()
        .filter(|j| j.agent_app_id() == Some(&agent))
        .collect();
    assert_eq!(placed.len(), 1);
}

#[tokio::test]
async fn a_dispatch_pass_refuses_a_caller_that_is_not_a_service() {
    let pl = two_nodes();
    let (app, jobs, registry) = dispatch_lab(vec![job(&pl)]).await;
    registry.connect(&agent_id());

    let err = JobControlRpc::dispatch_pending_jobs(&app.scoped(&alice()), None)
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(registry.dispatched_to().is_empty());
    assert!(jobs.rows()[0].agent_app_id().is_none());
}

#[tokio::test]
async fn the_end_of_a_stream_returns_only_its_own_jobs_to_the_pool() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let pl = two_nodes();
    let earlier = another_stream(&agent_id());
    let left = lab.jobs.place(&job(&pl), &earlier);
    let taken = lab.jobs.place(&job(&pl), &lab.stream);
    let running = lab.insert(&busy(&pl));

    let released = lab.release(&earlier).await.unwrap();
    let again = lab.release(&earlier).await.unwrap();

    assert_eq!((released, again), (1, 0));
    assert!(lab.jobs.row(left.id()).agent_app_id().is_none());
    assert_eq!(lab.jobs.row(taken.id()).agent_app_id(), Some(&agent_id()));
    assert_eq!(lab.jobs.stream_of(taken.id()), Some(lab.stream.id.clone()));
    assert_eq!(lab.jobs.row(running.id()).agent_app_id(), Some(&agent_id()));
    assert_eq!(lab.registry.wakes(), [None]);
}

#[tokio::test]
async fn only_an_agent_releases_or_reconciles_its_jobs() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let caller = alice();
    let alice = lab.app.scoped(&caller);

    let release = JobControlRpc::release_agent_jobs(&alice, lab.stream.id.clone()).await;
    let reconcile = JobControlRpc::reconcile_agent_jobs(&alice, vec![]).await;

    assert!(matches!(release, Err(DomainError::Forbidden(_))));
    assert!(matches!(reconcile, Err(DomainError::Forbidden(_))));
    assert!(lab.jobs.released().is_empty());
}

#[tokio::test]
async fn a_reconcile_trusts_the_list_of_the_agent() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let pl = two_nodes();
    let lost = lab.insert(&busy(&pl));
    let kept = lab.insert(&busy(&pl));
    let unstarted = lab.jobs.place(&job(&pl), &another_stream(&agent_id()));
    let elsewhere = lab.insert(&JobBuilder::new(&pl).agent(AppId::new("other")).build());
    let deleted = job(&pl);

    let changed = lab
        .reconcile(&[&kept, &unstarted, &elsewhere, &deleted])
        .await
        .unwrap();

    assert_eq!(changed, 1);
    assert_eq!(lab.jobs.row(lost.id()).status(), JobStatus::Orphaned);
    assert_eq!(lab.jobs.row(kept.id()).status(), JobStatus::Running);
    assert_eq!(lab.jobs.row(unstarted.id()).status(), JobStatus::Pending);
    assert_eq!(lab.tails.opened(), [kept.id().clone()]);
    let cancelled: Vec<JobId> = lab
        .registry
        .cancelled()
        .into_iter()
        .map(|(_, id)| id)
        .collect();
    assert_eq!(
        cancelled,
        [
            lost.id().clone(),
            unstarted.id().clone(),
            elsewhere.id().clone(),
            deleted.id().clone(),
        ]
    );
}

#[tokio::test]
async fn a_hello_leaves_the_job_placed_on_the_open_stream() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let taken = lab.jobs.place(&job(&two_nodes()), &lab.stream);

    let changed = lab.reconcile(&[]).await.unwrap();

    assert_eq!(changed, 0);
    assert_eq!(lab.jobs.row(taken.id()).agent_app_id(), Some(&agent_id()));
    assert!(lab.registry.cancelled().is_empty());
    assert!(lab.jobs.released().is_empty());
}

struct LogLab {
    app: TestCe,
    logs: Arc<StubLogs>,
    live: Arc<StubLive>,
}

async fn log_lab(permissions: Arc<dyn PermissionService>) -> LogLab {
    let pipeline = PipelineBuilder::for_project_id(ProjectId::new("p"))
        .nodes(vec![node("build", &[]), node("test", &["build"])])
        .build();
    let job = JobBuilder::new(&pipeline)
        .id(JobId::new("job-1"))
        .running(true)
        .agent(agent_id())
        .build()
        .apply_node_started(&NodeId::new("build").unwrap(), clock::now())
        .unwrap();
    let jobs = Arc::new(StubJobs::with(vec![job]));
    let logs = Arc::new(StubLogs::default());
    let live = Arc::new(StubLive::default());
    let app = build(
        permissions,
        jobs.clone(),
        jobs,
        Arc::new(StubRegistry::default()),
        logs.clone(),
        live.clone(),
        Arc::new(StubTails::default()),
    )
    .await;
    LogLab { app, logs, live }
}

struct LogQuery {
    job_id: JobId,
    node_id: Option<NodeId>,
    pagination: Option<PaginationParams>,
}

async fn list_job_logs<A: scylla_strata::JobControl>(
    app: &impl JobControlRpc<A>,
    q: LogQuery,
) -> DomainResult<PaginatedResult<JobLog>> {
    JobControlRpc::list_job_logs(app, q.job_id, q.node_id, q.pagination).await
}

struct LogBatch {
    job_id: JobId,
    logs: Vec<JobLog>,
}

async fn append_job_logs<A: scylla_strata::JobControl>(
    app: &impl JobControlRpc<A>,
    b: LogBatch,
) -> DomainResult<Vec<JobLog>> {
    JobControlRpc::append_job_logs(app, b.job_id, b.logs).await
}

fn list(node: Option<&str>) -> LogQuery {
    LogQuery {
        job_id: JobId::new("job-1"),
        node_id: node.map(|n| NodeId::new(n).unwrap()),
        pagination: None,
    }
}

fn append(job: &str, lines: &[&str]) -> LogBatch {
    let job_id = JobId::new(job);
    LogBatch {
        logs: lines
            .iter()
            .map(|line| job_log(&job_id, "build", line))
            .collect(),
        job_id,
    }
}

fn lines(logs: &[JobLog]) -> Vec<&str> {
    logs.iter().map(JobLog::line).collect()
}

async fn collect(tail: JobLogLiveStream) -> Vec<String> {
    tail.map(|l| l.unwrap().line().to_string()).collect().await
}

#[tokio::test]
async fn an_append_checks_once_stores_one_batch_and_publishes_each_line_in_order() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = log_lab(permissions.clone()).await;

    let stored = append_job_logs(
        &lab.app.scoped(&agent()),
        append("job-1", &["one", "two", "three"]),
    )
    .await
    .unwrap();

    assert_eq!(lines(&stored), ["one", "two", "three"]);
    assert_eq!(*lab.logs.reads.lock().unwrap(), vec!["insert"]);
    assert_eq!(
        lines(&lab.live.published.lock().unwrap()),
        ["one", "two", "three"]
    );
    assert_eq!(
        permissions.permissions(),
        vec![Permission::AppendJobLog(JobId::new("job-1"))]
    );
}

#[tokio::test]
async fn only_the_agent_of_the_job_appends_to_it() {
    let lab = log_lab(Arc::new(RecordingPermissionService::new())).await;
    let other = ctx(CallerContext::App(AppId::new("other")));

    let err = append_job_logs(&lab.app.scoped(&other), append("job-1", &["hello"]))
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::BusinessRule(_)));
    assert!(lab.logs.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_line_of_another_job_refuses_the_batch() {
    let lab = log_lab(Arc::new(RecordingPermissionService::new())).await;
    let mut batch = append("job-1", &["mine"]);
    batch
        .logs
        .push(job_log(&JobId::new("job-2"), "build", "theirs"));

    let err = append_job_logs(&lab.app.scoped(&agent()), batch)
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::Validation(_)));
    assert!(lab.logs.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_denied_append_stores_nothing() {
    let lab = log_lab(Arc::new(DenyingPermissionService::new())).await;

    let err = append_job_logs(&lab.app.scoped(&agent()), append("job-1", &["hello"]))
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.logs.rows.lock().unwrap().is_empty());
    assert!(lab.live.published.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_list_reads_every_node_or_the_one_it_names() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = log_lab(permissions.clone()).await;
    let job_id = JobId::new("job-1");
    lab.logs.rows.lock().unwrap().extend([
        job_log(&job_id, "build", "a"),
        job_log(&job_id, "test", "b"),
    ]);
    let caller = agent();
    let agent = lab.app.scoped(&caller);

    let all = list_job_logs(&agent, list(None)).await.unwrap();
    let one = list_job_logs(&agent, list(Some("build"))).await.unwrap();

    assert_eq!(all.items().len(), 2);
    assert_eq!(one.items().len(), 1);
    assert_eq!(*lab.logs.reads.lock().unwrap(), vec!["job", "node"]);
    assert_eq!(
        permissions.permissions(),
        vec![
            Permission::ReadJobLogs(job_id.clone()),
            Permission::ReadJobLogs(job_id.clone()),
            Permission::ReadJob(job_id),
        ]
    );
}

#[tokio::test]
async fn a_list_of_a_node_that_has_not_started_is_empty() {
    let lab = log_lab(Arc::new(RecordingPermissionService::new())).await;
    let job_id = JobId::new("job-1");
    lab.logs
        .rows
        .lock()
        .unwrap()
        .push(job_log(&job_id, "test", "early"));

    let page = list_job_logs(&lab.app.scoped(&agent()), list(Some("test")))
        .await
        .unwrap();

    assert!(page.items().is_empty());
    assert_eq!(page.metadata().total_count(), 0);
    assert!(lab.logs.reads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_tail_of_a_node_that_has_not_started_replays_nothing() {
    let lab = log_lab(Arc::new(RecordingPermissionService::new())).await;
    let job_id = JobId::new("job-1");
    lab.logs
        .rows
        .lock()
        .unwrap()
        .push(job_log(&job_id, "test", "early"));
    lab.live
        .lines
        .lock()
        .unwrap()
        .push(job_log(&job_id, "test", "live"));

    let tail = JobControlRpc::tail_job_logs(
        &lab.app.scoped(&agent()),
        job_id,
        Some(NodeId::new("test").unwrap()),
    )
    .await
    .unwrap();

    assert_eq!(collect(tail).await, vec!["live"]);
}

#[tokio::test]
async fn a_tail_replays_the_snapshot_then_only_unseen_live_lines() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = log_lab(permissions.clone()).await;
    let job_id = JobId::new("job-1");
    let at = Utc::now();
    let stored = JobLogBuilder::new(&job_id, "build", "old")
        .timestamp(at)
        .build();
    let newer = JobLogBuilder::new(&job_id, "build", "new")
        .timestamp(at + StdDuration::from_secs(1))
        .build();
    lab.logs.rows.lock().unwrap().push(stored.clone());
    lab.live
        .lines
        .lock()
        .unwrap()
        .extend([stored.clone(), newer.clone()]);

    let tail = JobControlRpc::tail_job_logs(&lab.app.scoped(&agent()), job_id.clone(), None)
        .await
        .unwrap();

    assert_eq!(collect(tail).await, vec!["old", "new"]);
    assert_eq!(
        permissions.permissions(),
        vec![Permission::ReadJobLogs(job_id)]
    );
}

#[tokio::test]
async fn a_denied_tail_never_subscribes() {
    let lab = log_lab(Arc::new(DenyingPermissionService::new())).await;

    let err = JobControlRpc::tail_job_logs(&lab.app.scoped(&agent()), JobId::new("job-1"), None)
        .await
        .err()
        .unwrap();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(*lab.live.subscribes.lock().unwrap(), 0);
}

async fn serve(app: TestCe) -> JobServiceClient<Channel> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Server::builder().add_service(JobServiceServer::new(Grpc::new(app)));
    tokio::spawn(router.serve_with_incoming(TcpIncoming::from(listener)));
    let channel = Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    JobServiceClient::new(channel)
}

fn bearer<T>(message: T, token: Option<&str>) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    if let Some(token) = token {
        request
            .metadata_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
    }
    request
}

fn get(id: Option<&JobId>) -> GetJobRequest {
    GetJobRequest {
        job_id: id.map(|id| PbJobId {
            value: id.to_string(),
        }),
    }
}

#[tokio::test]
async fn grpc_gets_a_job_and_refuses_before_it_parses() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let seeded = lab.insert(&job(&two_nodes()));
    let mut client = serve(lab.app).await;

    let got = client
        .get_job(bearer(get(Some(seeded.id())), Some("user:alice")))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        got.job.unwrap().job_id.unwrap().value,
        seeded.id().to_string()
    );

    let anonymous = client.get_job(bearer(get(None), None)).await.unwrap_err();
    assert_eq!(anonymous.code(), Code::Unauthenticated, "{anonymous:?}");
    assert_eq!(
        anonymous.message(),
        "Missing or invalid authorization token"
    );
    let tail = client
        .tail_job_logs(bearer(TailJobLogsRequest::default(), None))
        .await
        .err()
        .unwrap();
    assert_eq!(tail.code(), Code::Unauthenticated, "{tail:?}");
}

#[tokio::test]
async fn grpc_refuses_a_denied_caller() {
    let lab = lab(Arc::new(DenyingPermissionService::new())).await;
    let seeded = lab.insert(&busy(&two_nodes()));
    let jobs = lab.jobs.clone();
    let mut client = serve(lab.app).await;

    let denied = client
        .cancel_job(bearer(
            scylla_proto::job::v1::CancelJobRequest {
                job_id: Some(PbJobId {
                    value: seeded.id().to_string(),
                }),
            },
            Some("user:alice"),
        ))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::PermissionDenied, "{denied:?}");
    assert_eq!(jobs.row(seeded.id()).status(), JobStatus::Running);
}

#[tokio::test]
async fn grpc_tail_sends_the_history_then_the_live_lines_with_no_duplicate() {
    let job_id = JobId::new("job-1");
    let at = clock::now();
    let old = JobLogBuilder::new(&job_id, "build", "old")
        .timestamp(at)
        .build();
    let new = JobLogBuilder::new(&job_id, "build", "new")
        .timestamp(at + StdDuration::from_secs(1))
        .build();
    let logs = Arc::new(StubLogs::default());
    logs.rows.lock().unwrap().push(old.clone());
    let live = Arc::new(InMemoryJobLogStream::new());
    live.open(&job_id);
    let jobs = Arc::new(StubJobs::default());
    let app = build(
        Arc::new(RecordingPermissionService::new()),
        jobs.clone(),
        jobs,
        Arc::new(StubRegistry::default()),
        logs,
        live.clone(),
        Arc::new(StubTails::default()),
    )
    .await;
    let mut client = serve(app).await;

    let mut tail = client
        .tail_job_logs(bearer(
            TailJobLogsRequest {
                job_id: Some(PbJobId {
                    value: job_id.to_string(),
                }),
                node_id: None,
            },
            Some("user:alice"),
        ))
        .await
        .unwrap()
        .into_inner();
    let mut received = Vec::new();
    let first = tokio::time::timeout(StdDuration::from_secs(5), tail.message())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    received.push(first.log.unwrap().line);
    live.publish(&old);
    live.publish(&new);
    live.close(&job_id);
    while let Some(next) = tokio::time::timeout(StdDuration::from_secs(5), tail.message())
        .await
        .unwrap()
        .unwrap()
    {
        received.push(next.log.unwrap().line);
    }

    assert_eq!(received, ["old", "new"]);
}

#[tokio::test]
async fn a_reap_pass_ends_a_stranded_running_job_that_has_no_agent() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let pl = two_nodes();
    let lost = lab.insert(&stored(&busy(&pl), None));
    let kept = lab.insert(&busy(&pl));

    assert_eq!(lab.reap().await.unwrap(), 1);

    assert_eq!(lab.jobs.row(lost.id()).status(), JobStatus::Orphaned);
    assert_eq!(lab.jobs.row(kept.id()).status(), JobStatus::Running);
}

#[tokio::test]
async fn a_list_of_a_node_that_has_not_started_keeps_the_page_of_the_request() {
    let lab = log_lab(Arc::new(RecordingPermissionService::new())).await;
    let query = LogQuery {
        pagination: Some(PaginationParams::new(3, 5).unwrap()),
        ..list(Some("test"))
    };

    let page = list_job_logs(&lab.app.scoped(&agent()), query)
        .await
        .unwrap();

    assert!(page.items().is_empty());
    assert_eq!(page.metadata().page(), 3);
    assert_eq!(page.metadata().page_size(), 5);
    assert_eq!(page.metadata().total_count(), 0);
}

/// Resolves every node but the one named `broken`, as a secret deleted since the run.
struct BrokenSecret;

#[async_trait]
impl SecretResolver for BrokenSecret {
    async fn resolve(
        &self,
        project_id: &ProjectId,
        nodes: &[PipelineNode],
    ) -> DomainResult<Vec<DispatchNode>> {
        if nodes.iter().any(|n| n.id().as_str() == "broken") {
            return Err(DomainError::not_found("Secret", "gone"));
        }
        EchoResolver.resolve(project_id, nodes).await
    }
}

struct PassLab {
    app: TestCe,
    registry: Arc<StubRegistry>,
    jobs: Arc<StubJobs>,
    scopes: Arc<ScopesByAgent>,
    permissions: Arc<RecordingPermissionService>,
}

async fn pass_lab_with(
    registry: StubRegistry,
    jobs: Vec<Job>,
    scopes: ScopesByAgent,
    resolver: Arc<dyn SecretResolver>,
) -> PassLab {
    let registry = Arc::new(registry);
    let jobs = Arc::new(StubJobs::with(jobs));
    let scopes = Arc::new(scopes);
    let tails = Arc::new(StubTails::default());
    let permissions = Arc::new(RecordingPermissionService::new());
    let app = TestCe::builder(&Config::new())
        .provide(AccessModel {
            permissions: permissions.clone(),
            visibility: scopes.clone(),
        })
        .provide(JobDispatch(dispatcher(
            registry.clone(),
            jobs.clone(),
            tails.clone(),
        )))
        .provide_arc::<dyn AgentDispatch>(registry.clone())
        .provide_arc::<dyn SecretResolver>(resolver)
        .provide_arc::<dyn JobRepository>(jobs.clone())
        .provide_arc::<dyn JobLogRepository>(Arc::new(StubLogs::default()))
        .provide(JobLogTails(tails))
        .build()
        .await
        .unwrap();
    PassLab {
        app,
        registry,
        jobs,
        scopes,
        permissions,
    }
}

async fn pass_lab(connected: &[&str], jobs: Vec<Job>) -> PassLab {
    let registry = StubRegistry::accepting();
    for id in connected {
        registry.connect(&AppId::new(*id));
    }
    pass_lab_with(
        registry,
        jobs,
        ScopesByAgent::everything(),
        Arc::new(EchoResolver),
    )
    .await
}

impl PassLab {
    async fn pass(&self, agents: Option<Vec<AppId>>) -> DomainResult<Vec<Job>> {
        let dispatcher = ctx(CallerContext::Service(ServiceIdentity::job_dispatcher()));
        let placed =
            JobControlRpc::dispatch_pending_jobs(&self.app.scoped(&dispatcher), agents).await;
        assert!(
            self.permissions.checks().is_empty(),
            "a pass asks no permission"
        );
        placed
    }
}

fn a_pipeline() -> Pipeline {
    PipelineBuilder::for_project_id(ProjectId::new("p")).build()
}

#[tokio::test]
async fn a_pass_places_the_oldest_job_on_each_idle_agent_that_may_run_it() {
    let pl = a_pipeline();
    let (first, second, third) = (job(&pl), job(&pl), job(&pl));
    let lab = pass_lab(
        &["agent-1", "agent-2"],
        vec![first.clone(), second.clone(), third.clone()],
    )
    .await;

    let placed = lab.pass(None).await.unwrap();

    assert_eq!(placed.len(), 2);
    assert_eq!(
        lab.registry.dispatched_to(),
        [AppId::new("agent-1"), AppId::new("agent-2")]
    );
    assert_eq!(
        lab.registry.dispatched()[0].1.job_id,
        first.id().to_string()
    );
    assert_eq!(
        lab.registry.dispatched()[1].1.job_id,
        second.id().to_string()
    );
    assert!(lab.jobs.row(third.id()).agent_app_id().is_none());
    assert_eq!(lab.scopes.asked(), ["executeJob", "executeJob"]);
}

#[tokio::test]
async fn a_busy_agent_and_an_agent_that_sees_nothing_get_no_job() {
    let pl = a_pipeline();
    let busy = AppId::new("busy");
    let running = JobBuilder::new(&pl)
        .running(true)
        .agent(busy.clone())
        .build();
    let waiting = job(&pl);
    let registry = StubRegistry::accepting();
    registry.connect(&busy);
    registry.connect(&AppId::new("blind"));
    let lab = pass_lab_with(
        registry,
        vec![running, waiting.clone()],
        ScopesByAgent::new(
            HashMap::from([(busy.clone(), Visibility::All)]),
            Visibility::none(),
        ),
        Arc::new(EchoResolver),
    )
    .await;

    let placed = lab.pass(None).await.unwrap();

    assert!(placed.is_empty());
    assert!(lab.registry.dispatched().is_empty());
    assert!(lab.jobs.row(waiting.id()).agent_app_id().is_none());
}

#[tokio::test]
async fn an_agent_sees_only_the_scopes_of_its_grants() {
    let agent = AppId::new("agent-1");
    let registry = StubRegistry::accepting();
    registry.connect(&agent);
    let scoped = Visibility::Scoped {
        orgs: vec![OrganizationId::new("acme")],
        projects: vec![],
    };
    let lab = pass_lab_with(
        registry,
        vec![job(&a_pipeline())],
        ScopesByAgent::new(HashMap::from([(agent.clone(), scoped)]), Visibility::none()),
        Arc::new(EchoResolver),
    )
    .await;

    let placed = lab.pass(None).await.unwrap();

    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0].agent_app_id(), Some(&agent));
}

#[tokio::test]
async fn a_targeted_pass_touches_only_the_listed_agents_that_are_connected() {
    let pl = a_pipeline();
    let lab = pass_lab(&["agent-1", "agent-2"], vec![job(&pl), job(&pl)]).await;

    let placed = lab
        .pass(Some(vec![AppId::new("agent-2"), AppId::new("gone")]))
        .await
        .unwrap();

    assert_eq!(placed.len(), 1);
    assert_eq!(lab.registry.dispatched_to(), [AppId::new("agent-2")]);
}

#[tokio::test]
async fn a_failed_send_returns_the_job_to_the_pool_and_wakes_the_dispatcher() {
    let waiting = job(&a_pipeline());
    let registry = StubRegistry::closing();
    let stream = registry.connect(&AppId::new("agent-1"));
    let lab = pass_lab_with(
        registry,
        vec![waiting.clone()],
        ScopesByAgent::everything(),
        Arc::new(EchoResolver),
    )
    .await;

    let placed = lab.pass(None).await.unwrap();

    assert!(placed.is_empty());
    assert_eq!(lab.jobs.released(), [stream]);
    let row = lab.jobs.row(waiting.id());
    assert!(row.agent_app_id().is_none());
    assert_eq!(row.status(), JobStatus::Pending);
    assert_eq!(lab.registry.wakes(), [None]);
}

#[tokio::test]
async fn a_job_is_placed_on_the_stream_of_the_pass_and_only_that_stream_releases_it() {
    let waiting = job(&a_pipeline());
    let lab = pass_lab(&["agent-1"], vec![waiting.clone()]).await;
    let first = lab.registry.connected().remove(0);

    lab.pass(None).await.unwrap();
    let newer = lab.registry.connect(&AppId::new("agent-1"));
    let release = |stream: &AgentStream| {
        let agent = ctx(CallerContext::App(stream.agent.clone()));
        let id = stream.id.clone();
        let app = &lab.app;
        async move { JobControlRpc::release_agent_jobs(&app.scoped(&agent), id).await }
    };

    assert_eq!(lab.jobs.stream_of(waiting.id()), Some(first.id.clone()));
    assert_eq!(release(&newer).await.unwrap(), 0);
    assert_eq!(release(&first).await.unwrap(), 1);
    assert!(lab.jobs.row(waiting.id()).agent_app_id().is_none());
}

#[tokio::test]
async fn a_job_that_no_longer_dispatches_fails_and_the_agent_gets_the_next_one() {
    let broken = PipelineBuilder::for_project_id(ProjectId::new("p"))
        .nodes(vec![node("broken", &[])])
        .build();
    let (doomed, next) = (job(&broken), job(&a_pipeline()));
    let registry = StubRegistry::accepting();
    registry.connect(&AppId::new("agent-1"));
    let lab = pass_lab_with(
        registry,
        vec![doomed.clone(), next.clone()],
        ScopesByAgent::everything(),
        Arc::new(BrokenSecret),
    )
    .await;

    let placed = lab.pass(None).await.unwrap();

    assert_eq!(lab.jobs.row(doomed.id()).status(), JobStatus::Failed);
    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0].id(), next.id());
    assert_eq!(lab.registry.dispatched()[0].1.job_id, next.id().to_string());
}
