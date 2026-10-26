use async_trait::async_trait;
use scylla_auth::authz::{PermissionService, Visibility, VisibilityResolver};
use scylla_core::application::agent::{
    AgentDispatch, AgentStream, DispatchNode, JobDispatch as AgentJob,
};
use scylla_core::application::job::JobScope;
use scylla_core::application::pagination::{PaginatedResult, PaginationParams};
use scylla_core::application::{
    DispatchUseCases, JobLogLiveStream, JobLogStreamPort, JobRepository, PipelineRepository,
    ProjectRepository, SecretResolver,
};
use scylla_core::test_support::authz::{DenyingPermissionService, RecordingPermissionService};
use scylla_core::test_support::jobs::JobBuilder;
use scylla_core::test_support::pipelines::{PipelineBuilder, node};
use scylla_core::test_support::projects::ProjectBuilder;
use scylla_domain::domain::caller::{CallerContext, ServiceIdentity};
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{AppId, JobId, OrganizationId, PipelineId, ProjectId, UserId};
use scylla_domain::domain::job::{Job, JobLog, JobOrigin, JobStatus};
use scylla_domain::domain::permission::Permission;
use scylla_domain::domain::pipeline::{NodeId, Pipeline, PipelineName, PipelineNode, Shell, Step};
use scylla_domain::domain::project::Project;
use scylla_proto::common::v1::PipelineId as PbPipelineId;
use scylla_proto::pipeline::v1::RunPipelineRequest;
use scylla_strata::grpc::{PipelineServiceClient, PipelineServiceServer};
use scylla_strata::{AccessModel, CedarPermissions, JobDispatch, PipelinesRpc, ScyllaCtx};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use strata::{Config, Contextual, Edition};
use strata_tonic::Grpc;
use tonic::Code;
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Channel, Server};

#[strata::edition(modules(scylla_strata::Access, scylla_strata::Pipelines))]
pub struct TestBase;

#[strata::edition(extends = TestBase, overrides(scylla_strata::Access))]
pub struct TestCe;

#[strata::overrides]
impl scylla_strata::Access for TestCe {
    type Permissions = CedarPermissions;
}

#[derive(Default)]
struct StubPipelines {
    rows: Mutex<HashMap<PipelineId, Pipeline>>,
}

impl StubPipelines {
    fn page(&self, keep: impl Fn(&Pipeline) -> bool) -> DomainResult<PaginatedResult<Pipeline>> {
        let rows: Vec<Pipeline> = self
            .rows
            .lock()
            .unwrap()
            .values()
            .filter(|p| keep(p))
            .cloned()
            .collect();
        let total = rows.len() as u64;
        Ok(PaginatedResult::new(
            rows,
            &PaginationParams::default(),
            total,
        ))
    }
}

#[async_trait]
impl PipelineRepository for StubPipelines {
    async fn create(&self, pipeline: &Pipeline) -> DomainResult<Pipeline> {
        self.rows
            .lock()
            .unwrap()
            .insert(pipeline.id().clone(), pipeline.clone());
        Ok(pipeline.clone())
    }
    async fn find_by_id(&self, id: &PipelineId) -> DomainResult<Pipeline> {
        self.rows
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| DomainError::not_found("Pipeline", id))
    }
    async fn update(&self, pipeline: &Pipeline) -> DomainResult<Pipeline> {
        self.create(pipeline).await
    }
    async fn delete(&self, pipeline: &Pipeline) -> DomainResult<()> {
        self.rows.lock().unwrap().remove(pipeline.id());
        Ok(())
    }
    async fn list_all(
        &self,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Pipeline>> {
        self.page(|_| true)
    }
    async fn list_by_project(
        &self,
        project_id: &ProjectId,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Pipeline>> {
        self.page(|p| p.project_id() == project_id)
    }
    async fn list_by_organization(
        &self,
        _: &OrganizationId,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Pipeline>> {
        self.page(|_| false)
    }
}

struct OneProject(Project);

#[async_trait]
impl ProjectRepository for OneProject {
    async fn create(&self, _: &Project) -> DomainResult<Project> {
        unreachable!("a pipeline action writes no project")
    }
    async fn provision_with_owner(
        &self,
        _: &Project,
        _: &scylla_auth::authz::Grant,
    ) -> DomainResult<()> {
        unreachable!("a pipeline action writes no project")
    }
    async fn list_principals(
        &self,
        _: &ProjectId,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<UserId>> {
        unreachable!("a pipeline action reads one project")
    }
    async fn list_for_user(
        &self,
        _: &UserId,
        _: &str,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Project>> {
        unreachable!("a pipeline action reads one project")
    }
    async fn find_by_id(&self, id: &ProjectId) -> DomainResult<Project> {
        if id == self.0.id() {
            Ok(self.0.clone())
        } else {
            Err(DomainError::not_found("Project", id))
        }
    }
    async fn update(&self, _: &Project) -> DomainResult<Project> {
        unreachable!("a pipeline action writes no project")
    }
    async fn delete(&self, _: &Project) -> DomainResult<()> {
        unreachable!("a pipeline action writes no project")
    }
    async fn list_all(
        &self,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Project>> {
        unreachable!("a pipeline action reads one project")
    }
    async fn list_by_organization(
        &self,
        _: &OrganizationId,
        _: Option<&PaginationParams>,
        _: &Visibility,
    ) -> DomainResult<PaginatedResult<Project>> {
        unreachable!("a pipeline action reads one project")
    }
}

#[derive(Default)]
struct StubJobs {
    rows: Mutex<Vec<Job>>,
}

impl StubJobs {
    fn rows(&self) -> Vec<Job> {
        self.rows.lock().unwrap().clone()
    }
    fn insert(&self, job: &Job) -> Job {
        self.rows.lock().unwrap().push(job.clone());
        job.clone()
    }
    fn row(&self, id: &JobId) -> Job {
        self.rows
            .lock()
            .unwrap()
            .iter()
            .find(|j| j.id() == id)
            .cloned()
            .unwrap()
    }
}

#[async_trait]
impl JobRepository for StubJobs {
    async fn create(&self, job: &Job) -> DomainResult<Job> {
        Ok(self.insert(job))
    }
    async fn find_by_id(&self, _: &JobId) -> DomainResult<Job> {
        unreachable!("a pipeline action reads no job")
    }
    async fn update(&self, _: &Job) -> DomainResult<Job> {
        unreachable!("a pipeline action updates no job")
    }
    async fn delete(&self, _: &Job) -> DomainResult<()> {
        unreachable!("a pipeline action deletes no job")
    }
    async fn claim_next(
        &self,
        _: &AgentStream,
        _: &Visibility,
    ) -> DomainResult<Option<(Job, ProjectId)>> {
        unreachable!("a pipeline action places no job")
    }
    async fn release(&self, _: &AgentStream) -> DomainResult<u64> {
        unreachable!("a pipeline action releases no job")
    }
    async fn pending_streams(&self) -> DomainResult<Vec<AgentStream>> {
        unreachable!("a pipeline action places no job")
    }
    async fn active_jobs(&self, _: &[AppId]) -> DomainResult<Vec<(AppId, u32)>> {
        unreachable!("a pipeline action places no job")
    }
    async fn list_live(&self, scope: JobScope<'_>) -> DomainResult<Vec<Job>> {
        let rows = self.rows.lock().unwrap();
        Ok(rows
            .iter()
            .filter(|j| !j.is_terminal())
            .filter(|j| match scope {
                JobScope::Pipeline(id) => j.pipeline_id() == id,
                _ => true,
            })
            .cloned()
            .collect())
    }
    async fn list_running_on(&self, _: &AppId) -> DomainResult<Vec<Job>> {
        unreachable!("a pipeline action lists the live jobs only")
    }
    async fn list_stranded(
        &self,
        _: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
    ) -> DomainResult<Vec<Job>> {
        unreachable!("a pipeline action lists the live jobs only")
    }
    async fn list_all(&self, _: Option<&PaginationParams>) -> DomainResult<PaginatedResult<Job>> {
        unreachable!("a pipeline action lists the live jobs only")
    }
    async fn list_by_pipeline(
        &self,
        _: &PipelineId,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Job>> {
        unreachable!("a pipeline action lists the live jobs only")
    }
    async fn list_by_project(
        &self,
        _: &ProjectId,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Job>> {
        unreachable!("a pipeline action lists the live jobs only")
    }
    async fn list_by_organization(
        &self,
        _: &OrganizationId,
        _: Option<&AppId>,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Job>> {
        unreachable!("a pipeline action lists the live jobs only")
    }
}

#[derive(Default)]
struct StubRegistry {
    connected: Mutex<Vec<AppId>>,
    cancelled: Mutex<Vec<(AppId, JobId)>>,
    dispatched: Mutex<Vec<AppId>>,
    wakes: Mutex<Vec<Option<AppId>>>,
}

impl StubRegistry {
    fn connect(&self, agent: &AppId) {
        self.connected.lock().unwrap().push(agent.clone());
    }
    fn cancelled(&self) -> Vec<(AppId, JobId)> {
        self.cancelled.lock().unwrap().clone()
    }
    fn wakes(&self) -> Vec<Option<AppId>> {
        self.wakes.lock().unwrap().clone()
    }
}

impl AgentDispatch for StubRegistry {
    fn connected(&self) -> Vec<AgentStream> {
        Vec::new()
    }
    fn run(&self, stream: &AgentStream, _: AgentJob) -> DomainResult<()> {
        self.dispatched.lock().unwrap().push(stream.agent.clone());
        Ok(())
    }
    fn cancel(&self, agent: &AppId, job_id: &JobId) -> DomainResult<()> {
        if !self.connected.lock().unwrap().contains(agent) {
            return Err(DomainError::infrastructure("the agent takes no orders"));
        }
        self.cancelled
            .lock()
            .unwrap()
            .push((agent.clone(), job_id.clone()));
        Ok(())
    }
    fn disconnect(&self, _: &AppId) {}
    fn wake(&self, agent: Option<&AppId>) {
        self.wakes.lock().unwrap().push(agent.cloned());
    }
}

struct NoTails;

#[async_trait]
impl JobLogStreamPort for NoTails {
    fn open(&self, _: &JobId) {}
    fn publish(&self, _: &JobLog) {}
    fn close(&self, _: &JobId) {}
    async fn subscribe(&self, _: &JobId, _: Option<&NodeId>) -> DomainResult<JobLogLiveStream> {
        unreachable!("a pipeline action reads no log")
    }
}

struct SeeAll;

#[async_trait]
impl VisibilityResolver for SeeAll {
    async fn visible_scopes(&self, _: &CallerContext, _: &str) -> DomainResult<Visibility> {
        Ok(Visibility::All)
    }
}

struct EchoResolver;

#[async_trait]
impl SecretResolver for EchoResolver {
    async fn resolve(
        &self,
        _: &ProjectId,
        nodes: &[PipelineNode],
    ) -> DomainResult<Vec<DispatchNode>> {
        Ok(nodes
            .iter()
            .map(|n| DispatchNode {
                id: n.id().to_string(),
                deps: n.deps().iter().map(ToString::to_string).collect(),
                working_dir: n.working_dir().map(|w| w.as_str().to_string()),
                step: n.step().clone(),
                env: Vec::new(),
            })
            .collect())
    }
}

struct NoSecret;

#[async_trait]
impl SecretResolver for NoSecret {
    async fn resolve(&self, _: &ProjectId, _: &[PipelineNode]) -> DomainResult<Vec<DispatchNode>> {
        Err(DomainError::not_found("Secret", "TOKEN"))
    }
}

struct Lab<E> {
    app: E,
    pipelines: Arc<StubPipelines>,
    jobs: Arc<StubJobs>,
    registry: Arc<StubRegistry>,
}

fn project_id() -> ProjectId {
    ProjectId::new("proj-1")
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

async fn lab_of<E: Edition>(
    permissions: Arc<dyn PermissionService>,
    secrets: Arc<dyn SecretResolver>,
) -> Lab<E> {
    let pipelines = Arc::new(StubPipelines::default());
    let jobs = Arc::new(StubJobs::default());
    let registry = Arc::new(StubRegistry::default());
    let project = ProjectBuilder::for_org_id(OrganizationId::new("acme"), "rocket")
        .id(project_id())
        .build();
    let dispatch = Arc::new(DispatchUseCases::new(
        registry.clone(),
        jobs.clone(),
        Arc::new(NoTails),
    ));
    let app = E::builder(&Config::new())
        .provide(AccessModel {
            permissions,
            visibility: Arc::new(SeeAll),
        })
        .provide(JobDispatch(dispatch))
        .provide_arc::<dyn PipelineRepository>(pipelines.clone())
        .provide_arc::<dyn ProjectRepository>(Arc::new(OneProject(project)))
        .provide_arc::<dyn JobRepository>(jobs.clone())
        .provide_arc::<dyn SecretResolver>(secrets)
        .build()
        .await
        .unwrap();
    Lab {
        app,
        pipelines,
        jobs,
        registry,
    }
}

async fn lab(permissions: Arc<dyn PermissionService>) -> Lab<TestCe> {
    lab_of(permissions, Arc::new(EchoResolver)).await
}

impl<E: scylla_strata::Pipelines + Contextual<Ctx = ScyllaCtx>> Lab<E> {
    async fn create(&self) -> DomainResult<Pipeline> {
        let (project_id, name, nodes) = create();
        PipelinesRpc::create_pipeline(&self.app.scoped(&alice()), project_id, name, nodes).await
    }

    fn seed(&self) -> Pipeline {
        self.seed_with(PipelineBuilder::for_project_id(project_id()).build())
    }

    fn seed_with(&self, pipeline: Pipeline) -> Pipeline {
        self.pipelines
            .rows
            .lock()
            .unwrap()
            .insert(pipeline.id().clone(), pipeline.clone());
        pipeline
    }

    async fn run_as(&self, caller: ScyllaCtx, pipeline: &Pipeline) -> DomainResult<Job> {
        PipelinesRpc::run_pipeline(&self.app.scoped(&caller), pipeline.id().clone()).await
    }

    async fn run(&self, pipeline: &Pipeline) -> DomainResult<Job> {
        self.run_as(alice(), pipeline).await
    }

    async fn update(
        &self,
        id: PipelineId,
        name: Option<PipelineName>,
        nodes: Option<Vec<PipelineNode>>,
    ) -> DomainResult<Pipeline> {
        PipelinesRpc::update_pipeline(&self.app.scoped(&alice()), id, name, nodes).await
    }
}

fn create() -> (ProjectId, PipelineName, Vec<PipelineNode>) {
    (
        project_id(),
        PipelineName::new("build").unwrap(),
        vec![node("a", &[])],
    )
}

#[tokio::test]
async fn a_create_checks_the_permission_on_the_project_then_writes() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;

    let pipeline = lab.create().await.unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::CreatePipeline(project_id())]
    );
    assert!(
        lab.pipelines
            .rows
            .lock()
            .unwrap()
            .contains_key(pipeline.id())
    );
}

#[tokio::test]
async fn a_create_in_an_unknown_project_is_not_found_and_writes_nothing() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;

    let (_, name, nodes) = create();
    let err = PipelinesRpc::create_pipeline(
        &lab.app.scoped(&alice()),
        ProjectId::new("ghost"),
        name,
        nodes,
    )
    .await
    .unwrap_err();

    assert!(matches!(err, DomainError::NotFound(_)));
    assert!(lab.pipelines.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_denied_caller_writes_nothing() {
    let lab = lab(Arc::new(DenyingPermissionService::new())).await;

    let err = lab.create().await.unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.pipelines.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_update_checks_its_permission_and_changes_only_what_is_set() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let seeded = lab.seed();

    let updated = lab
        .update(
            seeded.id().clone(),
            Some(PipelineName::new("deploy").unwrap()),
            None,
        )
        .await
        .unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::UpdatePipeline(seeded.id().clone())]
    );
    assert_eq!(updated.name().as_str(), "deploy");
    assert_eq!(updated.nodes().len(), seeded.nodes().len());
}

#[tokio::test]
async fn a_delete_checks_its_permission_removes_the_row_then_stops_its_live_jobs() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let seeded = lab.seed();
    let agent = AppId::new("agent-1");
    lab.registry.connect(&agent);
    let running = lab.jobs.insert(
        &JobBuilder::new(&seeded)
            .running(true)
            .agent(agent.clone())
            .build(),
    );
    lab.jobs.insert(
        &JobBuilder::new(&seeded)
            .terminated(JobStatus::Completed)
            .agent(agent.clone())
            .build(),
    );

    PipelinesRpc::delete_pipeline(&lab.app.scoped(&alice()), seeded.id().clone())
        .await
        .unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::DeletePipeline(seeded.id().clone())]
    );
    assert!(lab.pipelines.rows.lock().unwrap().is_empty());
    assert_eq!(lab.registry.cancelled(), [(agent, running.id().clone())]);
}

#[tokio::test]
async fn a_get_and_a_project_list_check_their_permissions() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let seeded = lab.seed();
    let caller = alice();
    let alice = lab.app.scoped(&caller);

    let got = PipelinesRpc::get_pipeline(&alice, seeded.id().clone())
        .await
        .unwrap();
    let page = PipelinesRpc::list_project_pipelines(&alice, project_id(), None)
        .await
        .unwrap();

    assert_eq!(got.id(), seeded.id());
    assert_eq!(page.into_parts().0.len(), 1);
    assert_eq!(
        permissions.permissions(),
        vec![
            Permission::ReadPipeline(seeded.id().clone()),
            Permission::ListPipelinesByProject(project_id())
        ]
    );
}

#[tokio::test]
async fn a_run_by_a_user_stores_a_pending_job_and_wakes_the_dispatcher() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let seeded = lab.seed();
    lab.registry.connect(&AppId::new("agent-1"));

    let job = lab.run(&seeded).await.unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::RunPipeline(seeded.id().clone())]
    );
    assert_eq!(
        job.origin(),
        &JobOrigin::Human {
            user_id: UserId::new("alice")
        }
    );
    assert!(job.inputs().is_empty());
    assert_eq!(job.status(), JobStatus::Pending);
    assert!(job.agent_app_id().is_none());
    assert_eq!(lab.jobs.rows().len(), 1);
    assert!(lab.registry.dispatched.lock().unwrap().is_empty());
    assert_eq!(lab.registry.wakes(), [None]);
}

#[tokio::test]
async fn a_job_keeps_the_nodes_of_its_pipeline_when_the_pipeline_changes() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let seeded = lab.seed_with(
        PipelineBuilder::for_project_id(project_id())
            .nodes(vec![node("old1", &[]), node("old2", &["old1"])])
            .build(),
    );
    let job = lab.run(&seeded).await.unwrap();

    lab.update(seeded.id().clone(), None, Some(vec![node("new", &[])]))
        .await
        .unwrap();

    let stored = lab.jobs.row(job.id());
    let ids: Vec<&str> = stored.nodes().iter().map(|n| n.id().as_str()).collect();
    assert_eq!(ids, ["old1", "old2"]);
}

#[tokio::test]
async fn a_run_whose_secret_does_not_resolve_stores_nothing() {
    let lab: Lab<TestCe> = lab_of(
        Arc::new(RecordingPermissionService::new()),
        Arc::new(NoSecret),
    )
    .await;
    let seeded = lab.seed();

    let err = lab.run(&seeded).await.unwrap_err();

    assert!(matches!(err, DomainError::NotFound(_)));
    assert!(lab.jobs.rows().is_empty());
    assert!(lab.registry.wakes().is_empty());
}

#[tokio::test]
async fn a_run_too_large_to_dispatch_stores_nothing() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let script = Step::script("x".repeat(1024 * 1024), Shell::Sh).unwrap();
    let nodes = (0..9)
        .map(|i| {
            PipelineNode::new(
                NodeId::new(format!("n{i}")).unwrap(),
                vec![],
                script.clone(),
                None,
                vec![],
            )
        })
        .collect();
    let seeded = lab.seed_with(
        PipelineBuilder::for_project_id(project_id())
            .nodes(nodes)
            .build(),
    );

    let err = lab.run(&seeded).await.unwrap_err();

    assert!(matches!(err, DomainError::Validation(_)));
    assert!(lab.jobs.rows().is_empty());
}

#[tokio::test]
async fn a_run_by_a_service_caller_is_forbidden_and_creates_no_job() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let seeded = lab.seed();

    let err = lab
        .run_as(
            ctx(CallerContext::Service(ServiceIdentity::recorder())),
            &seeded,
        )
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.jobs.rows().is_empty());
}

#[tokio::test]
async fn a_denied_run_creates_no_job() {
    let lab = lab(Arc::new(DenyingPermissionService::new())).await;
    let seeded = lab.seed();

    let err = lab.run(&seeded).await.unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.jobs.rows().is_empty());
}

async fn serve(app: TestCe) -> PipelineServiceClient<Channel> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Server::builder().add_service(PipelineServiceServer::new(Grpc::new(app)));
    tokio::spawn(router.serve_with_incoming(TcpIncoming::from(listener)));
    let channel = Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    PipelineServiceClient::new(channel)
}

fn run_request(id: &PipelineId, bearer: Option<&str>) -> tonic::Request<RunPipelineRequest> {
    let mut request = tonic::Request::new(RunPipelineRequest {
        pipeline_id: Some(PbPipelineId {
            value: id.to_string(),
        }),
    });
    if let Some(bearer) = bearer {
        request
            .metadata_mut()
            .insert("authorization", bearer.parse().unwrap());
    }
    request
}

#[tokio::test]
async fn grpc_runs_a_pipeline_and_returns_the_job_id() {
    let lab: Lab<TestCe> = lab_of(
        Arc::new(RecordingPermissionService::new()),
        Arc::new(EchoResolver),
    )
    .await;
    let seeded = lab.seed();
    let jobs = lab.jobs.clone();
    let mut client = serve(lab.app).await;

    let ran = client
        .run_pipeline(run_request(seeded.id(), Some("Bearer user:alice")))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(ran.job_id.unwrap().value, jobs.rows()[0].id().to_string());

    let anonymous = client
        .run_pipeline(run_request(seeded.id(), None))
        .await
        .unwrap_err();
    assert_eq!(anonymous.code(), Code::Unauthenticated, "{anonymous:?}");
    assert_eq!(
        anonymous.message(),
        "Missing or invalid authorization token"
    );
    let unparsed = client
        .run_pipeline(tonic::Request::new(RunPipelineRequest {
            pipeline_id: None,
        }))
        .await
        .unwrap_err();
    assert_eq!(unparsed.code(), Code::Unauthenticated, "{unparsed:?}");
}

#[tokio::test]
async fn grpc_refuses_a_denied_caller() {
    let lab: Lab<TestCe> = lab_of(
        Arc::new(DenyingPermissionService::new()),
        Arc::new(EchoResolver),
    )
    .await;
    let seeded = lab.seed();
    let jobs = lab.jobs.clone();
    let mut client = serve(lab.app).await;

    let denied = client
        .run_pipeline(run_request(seeded.id(), Some("Bearer user:alice")))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::PermissionDenied, "{denied:?}");
    assert!(jobs.rows().is_empty());
}
