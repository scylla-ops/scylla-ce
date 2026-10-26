use async_trait::async_trait;
use scylla_auth::authz::{
    Grant, ORGANIZATION_AGENT_ROLE, PermissionService, Principal, Scope, Visibility,
    VisibilityResolver,
};
use scylla_core::application::agent::{AgentDispatch, CreatedAgent};
use scylla_core::application::pagination::{PaginatedResult, PaginationParams};
use scylla_core::application::{
    AgentRepository, AgentStats, AppCredentialRepository, AppRepository, AppTokenRepository,
    DispatchUseCases, HashService, JobLogRepository, JobRepository, PrincipalAccess,
    SecretResolver,
};
use scylla_core::infrastructure::InMemoryAgentRegistry;
use scylla_core::test_support::authz::{DenyingPermissionService, RecordingPermissionService};
use scylla_core::test_support::jobs::JobBuilder;
use scylla_core::test_support::pipelines::{PipelineBuilder, node};
use scylla_core::test_support::stubs::{
    EchoResolver, StubHash, StubJobs, StubRegistry, StubTails, dispatcher,
};
use scylla_domain::domain::agent::{Agent, AgentHost};
use scylla_domain::domain::app::{
    App, AppCredential, AppName, AppSecretLabel, AppToken, TRIGGER_RUNNER_APP_NAME,
};
use scylla_domain::domain::caller::{CallerContext, ServiceIdentity};
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{
    AppCredentialId, AppId, JobId, OrganizationId, ProjectId, UserId,
};
use scylla_domain::domain::job::{Job, JobLog, JobStatus as Phase};
use scylla_domain::domain::permission::Permission;
use scylla_domain::domain::pipeline::NodeId;
use scylla_proto::agent::v1::job_status::{Event, JobCompleted, JobStarted};
use scylla_proto::agent::v1::{
    AgentDown, AgentHello, AgentUp, CancelJob as PbCancel, CreateAgentRequest, DeleteAgentRequest,
    GetAgentRequest, JobLogLine, JobStatus, agent_down, agent_up,
};
use scylla_proto::common::v1 as common;
use scylla_strata::grpc::{
    AgentAdminServiceClient, AgentAdminServiceServer, AgentServiceClient, AgentServiceServer,
};
use scylla_strata::{
    AccessModel, AgentStreams, AgentsRpc, AppsRpc, CedarPermissions, JobControlRpc, JobDispatch,
    JobLogTails, ScyllaCtx,
};
use sqlx::types::chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use strata::{Config, Contextual, Edition, InFlight};
use strata_tonic::Grpc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Channel, Server};
use tonic::{Code, Streaming};

#[strata::edition(modules(
    scylla_strata::Access,
    scylla_strata::Apps,
    scylla_strata::Agents,
    scylla_strata::JobControl
))]
pub struct TestBase;

#[strata::edition(extends = TestBase, overrides(scylla_strata::Access))]
pub struct TestCe;

#[strata::overrides]
impl scylla_strata::Access for TestCe {
    type Permissions = CedarPermissions;
}

#[derive(Default)]
struct StubApps {
    rows: Mutex<HashMap<AppId, App>>,
    credentials: Mutex<Vec<AppCredential>>,
    grants: Mutex<Vec<Grant>>,
    agents: Arc<StubAgents>,
}

#[async_trait]
impl AppRepository for StubApps {
    async fn create_app(&self, _: &App, _: &AppCredential) -> DomainResult<()> {
        unreachable!("an agent is provisioned, not created")
    }
    async fn provision_agent(
        &self,
        app: &App,
        credential: &AppCredential,
        agent: &Agent,
        grant: &Grant,
    ) -> DomainResult<()> {
        self.rows
            .lock()
            .unwrap()
            .insert(app.id().clone(), app.clone());
        self.credentials.lock().unwrap().push(credential.clone());
        self.grants.lock().unwrap().push(grant.clone());
        self.agents.rows.lock().unwrap().push(agent.clone());
        Ok(())
    }
    async fn provision(&self, _: &App, _: &Grant) -> DomainResult<()> {
        unreachable!("an agent is provisioned with its agent row")
    }
    async fn find_by_id(&self, id: &AppId) -> DomainResult<App> {
        self.rows
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| DomainError::not_found("App", id))
    }
    async fn find_trigger_runner(&self, _: &OrganizationId) -> DomainResult<Option<AppId>> {
        unreachable!("no trigger in an agent action")
    }
    async fn list_by_organization(&self, _: &OrganizationId) -> DomainResult<Vec<App>> {
        unreachable!("agents are listed through the agent repository")
    }
    async fn set_active(&self, _: &AppId, _: bool) -> DomainResult<()> {
        unreachable!("no activation in an agent action")
    }
    async fn delete(&self, id: &AppId) -> DomainResult<()> {
        self.rows.lock().unwrap().remove(id);
        self.agents
            .rows
            .lock()
            .unwrap()
            .retain(|a| a.app_id() != id);
        Ok(())
    }
    async fn list_access(&self, _: &AppId) -> DomainResult<Vec<PrincipalAccess>> {
        unreachable!("no app access in an agent action")
    }
}

#[derive(Default)]
struct StubAgents {
    rows: Mutex<Vec<Agent>>,
    organizations: Mutex<HashMap<AppId, OrganizationId>>,
    touched: Mutex<Vec<AppId>>,
    hosts: Mutex<Vec<(AppId, AgentHost)>>,
}

impl StubAgents {
    fn touches(&self) -> usize {
        self.touched.lock().unwrap().len()
    }
}

#[async_trait]
impl AgentRepository for StubAgents {
    async fn find_by_app_id(&self, app_id: &AppId) -> DomainResult<Agent> {
        self.rows
            .lock()
            .unwrap()
            .iter()
            .find(|a| a.app_id() == app_id)
            .cloned()
            .ok_or_else(|| DomainError::not_found("Agent", app_id))
    }
    async fn list_by_organization(&self, org_id: &OrganizationId) -> DomainResult<Vec<Agent>> {
        let organizations = self.organizations.lock().unwrap();
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .filter(|a| organizations.get(a.app_id()) == Some(org_id))
            .cloned()
            .collect())
    }
    async fn touch_last_seen(&self, app_id: &AppId, _: DateTime<Utc>) -> DomainResult<()> {
        self.touched.lock().unwrap().push(app_id.clone());
        Ok(())
    }
    async fn record_host(&self, app_id: &AppId, host: &AgentHost) -> DomainResult<()> {
        self.hosts
            .lock()
            .unwrap()
            .push((app_id.clone(), host.clone()));
        Ok(())
    }
    async fn agent_stats(&self, _: &AppId) -> DomainResult<AgentStats> {
        Ok(AgentStats {
            total: 3,
            ..AgentStats::default()
        })
    }
}

struct NoCredentials;

#[async_trait]
impl AppCredentialRepository for NoCredentials {
    async fn create(&self, _: &AppCredential) -> DomainResult<()> {
        unreachable!("an agent writes its secret with its app")
    }
    async fn find_by_id(&self, _: &AppCredentialId) -> DomainResult<AppCredential> {
        unreachable!("no secret read in an agent action")
    }
    async fn list_by_app(&self, _: &AppId) -> DomainResult<Vec<AppCredential>> {
        unreachable!("no secret read in an agent action")
    }
    async fn list_enabled_by_app(&self, _: &AppId) -> DomainResult<Vec<AppCredential>> {
        unreachable!("no secret read in an agent action")
    }
    async fn set_enabled(&self, _: &AppCredentialId, _: bool) -> DomainResult<()> {
        unreachable!("no secret write in an agent action")
    }
    async fn set_label(&self, _: &AppCredentialId, _: &AppSecretLabel) -> DomainResult<()> {
        unreachable!("no secret write in an agent action")
    }
    async fn delete(&self, _: &AppCredentialId) -> DomainResult<()> {
        unreachable!("no secret write in an agent action")
    }
}

struct NoTokens;

#[async_trait]
impl AppTokenRepository for NoTokens {
    async fn create(&self, _: &AppToken) -> DomainResult<()> {
        unreachable!("no token in an agent action")
    }
    async fn find_by_token(&self, _: &str) -> DomainResult<AppToken> {
        unreachable!("no token in an agent action")
    }
}

#[derive(Default)]
struct Lines(Mutex<Vec<JobLog>>);

impl Lines {
    fn lines(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|l| l.line().to_string())
            .collect()
    }
}

#[async_trait]
impl JobLogRepository for Lines {
    async fn create_many(&self, logs: &[JobLog]) -> DomainResult<()> {
        self.0.lock().unwrap().extend_from_slice(logs);
        Ok(())
    }
    async fn list_by_job(
        &self,
        _: &JobId,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<JobLog>> {
        unreachable!("the agent stream lists no line")
    }
    async fn list_by_job_and_node(
        &self,
        _: &JobId,
        _: &NodeId,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<JobLog>> {
        unreachable!("the agent stream lists no line")
    }
    async fn list_all_by_job(&self, _: &JobId, _: Option<&NodeId>) -> DomainResult<Vec<JobLog>> {
        unreachable!("the agent stream lists no line")
    }
}

struct SeeAll;

#[async_trait]
impl VisibilityResolver for SeeAll {
    async fn visible_scopes(&self, _: &CallerContext, _: &str) -> DomainResult<Visibility> {
        Ok(Visibility::All)
    }
}

struct Parts {
    permissions: Arc<dyn PermissionService>,
    registry: Arc<dyn AgentDispatch>,
    streams: Arc<InMemoryAgentRegistry>,
    jobs: Arc<StubJobs>,
    tails: Arc<StubTails>,
    dispatch: Arc<DispatchUseCases>,
}

struct Lab {
    app: Arc<TestCe>,
    apps: Arc<StubApps>,
    agents: Arc<StubAgents>,
    jobs: Arc<StubJobs>,
    hash: Arc<StubHash>,
    lines: Arc<Lines>,
    tails: Arc<StubTails>,
}

fn quiet_registry() -> Arc<InMemoryAgentRegistry> {
    Arc::new(InMemoryAgentRegistry::new(mpsc::unbounded_channel().0))
}

async fn lab_of(parts: Parts) -> Lab {
    let agents = Arc::new(StubAgents::default());
    let apps = Arc::new(StubApps {
        agents: agents.clone(),
        ..StubApps::default()
    });
    let hash = Arc::new(StubHash::secrets());
    let lines = Arc::new(Lines::default());
    let app = TestCe::builder(&Config::new())
        .provide(AccessModel {
            permissions: parts.permissions,
            visibility: Arc::new(SeeAll),
        })
        .provide(JobDispatch(parts.dispatch.clone()))
        .provide(AgentStreams(parts.streams))
        .provide_arc::<dyn AppRepository>(apps.clone())
        .provide_arc::<dyn AppCredentialRepository>(Arc::new(NoCredentials))
        .provide_arc::<dyn AppTokenRepository>(Arc::new(NoTokens))
        .provide_arc::<dyn HashService>(hash.clone())
        .provide_arc::<dyn AgentDispatch>(parts.registry)
        .provide_arc::<dyn SecretResolver>(Arc::new(EchoResolver))
        .provide_arc::<dyn AgentRepository>(agents.clone())
        .provide_arc::<dyn JobRepository>(parts.jobs.clone())
        .provide_arc::<dyn JobLogRepository>(lines.clone())
        .provide(JobLogTails(parts.tails.clone()))
        .build()
        .await
        .unwrap();
    Lab {
        app: Arc::new(app),
        apps,
        agents,
        jobs: parts.jobs,
        hash,
        lines,
        tails: parts.tails,
    }
}

struct AdminLab {
    lab: Lab,
    registry: Arc<StubRegistry>,
}

async fn admin_lab(permissions: Arc<dyn PermissionService>) -> AdminLab {
    let registry = Arc::new(StubRegistry::default());
    let jobs = Arc::new(StubJobs::default());
    let tails = Arc::new(StubTails::default());
    let dispatch = dispatcher(registry.clone(), jobs.clone(), tails.clone());
    let lab = lab_of(Parts {
        permissions,
        registry: registry.clone(),
        streams: quiet_registry(),
        jobs,
        tails,
        dispatch,
    })
    .await;
    AdminLab { lab, registry }
}

async fn stream_lab(jobs: Vec<Job>) -> (Lab, Arc<InMemoryAgentRegistry>) {
    let registry = quiet_registry();
    let jobs = Arc::new(StubJobs::with(jobs));
    let tails = Arc::new(StubTails::default());
    let dispatch = dispatcher(registry.clone(), jobs.clone(), tails.clone());
    let lab = lab_of(Parts {
        permissions: Arc::new(RecordingPermissionService::new()),
        registry: registry.clone(),
        streams: registry.clone(),
        jobs,
        tails,
        dispatch,
    })
    .await;
    (lab, registry)
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

fn org() -> OrganizationId {
    OrganizationId::new("acme")
}

impl Lab {
    async fn create(&self) -> DomainResult<CreatedAgent> {
        let created = AgentsRpc::create_agent(
            &self.app.scoped(&alice()),
            org(),
            AppName::new("runner-1").unwrap(),
            None,
        )
        .await?;
        self.agents
            .organizations
            .lock()
            .unwrap()
            .insert(created.app.id().clone(), org());
        Ok(created)
    }
}

#[tokio::test]
async fn a_create_provisions_the_app_its_secret_its_agent_row_and_its_grant() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let AdminLab { lab, .. } = admin_lab(permissions.clone()).await;

    let created = lab.create().await.unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::CreateAgent(org())]
    );
    assert!(!created.secret.as_str().is_empty());
    let id = created.app.id().clone();
    assert!(lab.apps.rows.lock().unwrap().contains_key(&id));
    let credentials = lab.apps.credentials.lock().unwrap();
    assert_eq!(credentials.len(), 1);
    assert_eq!(credentials[0].label().as_str(), "runner-1");
    assert_eq!(lab.agents.rows.lock().unwrap()[0].app_id(), &id);
    let grants = lab.apps.grants.lock().unwrap();
    assert_eq!(grants[0].principal, Principal::App(id));
    assert_eq!(grants[0].role.as_str(), ORGANIZATION_AGENT_ROLE);
    assert_eq!(grants[0].scope, Scope::Organization(org()));
}

#[tokio::test]
async fn a_create_with_a_secret_label_names_the_first_secret_with_it() {
    let AdminLab { lab, .. } = admin_lab(Arc::new(RecordingPermissionService::new())).await;

    AgentsRpc::create_agent(
        &lab.app.scoped(&alice()),
        org(),
        AppName::new("runner-1").unwrap(),
        Some(AppSecretLabel::new("build-01.example.com").unwrap()),
    )
    .await
    .unwrap();

    let credentials = lab.apps.credentials.lock().unwrap();
    assert_eq!(credentials[0].label().as_str(), "build-01.example.com");
}

#[tokio::test]
async fn a_denied_create_never_hashes_or_persists() {
    let AdminLab { lab, .. } = admin_lab(Arc::new(DenyingPermissionService::new())).await;

    let err = lab.create().await.err().unwrap();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(lab.hash.hashed(), 0);
    assert!(lab.apps.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn reads_check_their_permission_and_join_the_registry_and_the_placed_jobs() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let AdminLab { lab, registry } = admin_lab(permissions.clone()).await;
    let id = lab.create().await.unwrap().app.id().clone();
    registry.connect(&id);
    let pipeline = PipelineBuilder::for_project_id(ProjectId::new("p")).build();
    lab.jobs.insert(
        &JobBuilder::new(&pipeline)
            .running(true)
            .agent(id.clone())
            .build(),
    );
    let caller = alice();
    let alice = lab.app.scoped(&caller);

    let views = AgentsRpc::list_agents(&alice, org()).await.unwrap();
    let view = AgentsRpc::get_agent(&alice, id.clone()).await.unwrap();
    let stats = AgentsRpc::get_agent_stats(&alice, id.clone())
        .await
        .unwrap();

    assert_eq!(views.len(), 1);
    assert!(views[0].connected);
    assert_eq!(views[0].in_flight, 1);
    assert_eq!(view.app.id(), &id);
    assert!(view.connected);
    assert_eq!(view.in_flight, 1);
    assert_eq!(stats.total, 3);
    assert_eq!(
        permissions.permissions()[1..],
        [
            Permission::ListAgents(org()),
            Permission::ReadApp(id.clone()),
            Permission::ReadAppStats(id),
        ]
    );
}

#[tokio::test]
async fn a_get_of_an_app_that_is_not_an_agent_is_not_found() {
    let AdminLab { lab, .. } = admin_lab(Arc::new(RecordingPermissionService::new())).await;

    let err = AgentsRpc::get_agent(&lab.app.scoped(&alice()), AppId::new("ghost"))
        .await
        .err()
        .unwrap();

    assert!(matches!(err, DomainError::NotFound(_)));
}

#[tokio::test]
async fn the_stats_of_an_unknown_app_are_not_found() {
    let AdminLab { lab, .. } = admin_lab(Arc::new(RecordingPermissionService::new())).await;

    let err = AgentsRpc::get_agent_stats(&lab.app.scoped(&alice()), AppId::new("ghost"))
        .await
        .err()
        .unwrap();

    assert!(matches!(err, DomainError::NotFound(_)));
}

#[tokio::test]
async fn the_runner_name_is_refused() {
    let AdminLab { lab, .. } = admin_lab(Arc::new(RecordingPermissionService::new())).await;

    let err = AgentsRpc::create_agent(
        &lab.app.scoped(&alice()),
        org(),
        AppName::new(TRIGGER_RUNNER_APP_NAME).unwrap(),
        None,
    )
    .await
    .err()
    .unwrap();

    assert!(matches!(err, DomainError::Validation(_)));
    assert_eq!(lab.hash.hashed(), 0);
}

fn host() -> AgentHost {
    AgentHost {
        version: "1.0.0".to_string(),
        os: "linux".to_string(),
        arch: "x86_64".to_string(),
        hostname: "runner-1".to_string(),
        cpu_count: Some(4),
        total_memory_mb: None,
        reported_at: Utc::now(),
    }
}

#[tokio::test]
async fn an_agent_touches_and_reports_its_own_row_with_no_permission_asked() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let AdminLab { lab, .. } = admin_lab(permissions.clone()).await;
    let caller = ctx(CallerContext::App(AppId::new("agent-1")));
    let agent = lab.app.scoped(&caller);

    AgentsRpc::touch_agent(&agent).await.unwrap();
    let reported = AgentsRpc::record_agent_host(&agent, host()).await.unwrap();

    assert!(permissions.permissions().is_empty());
    assert_eq!(
        lab.agents.touched.lock().unwrap().as_slice(),
        [AppId::new("agent-1")]
    );
    assert_eq!(
        lab.agents.hosts.lock().unwrap().as_slice(),
        [(AppId::new("agent-1"), reported)]
    );
}

#[tokio::test]
async fn only_an_agent_reports_its_own_state() {
    let AdminLab { lab, .. } = admin_lab(Arc::new(RecordingPermissionService::new())).await;
    let caller = alice();
    let alice = lab.app.scoped(&caller);

    let touch = AgentsRpc::touch_agent(&alice).await;
    let report = AgentsRpc::record_agent_host(&alice, host()).await;

    assert!(matches!(touch, Err(DomainError::Forbidden(_))));
    assert!(matches!(report, Err(DomainError::Forbidden(_))));
    assert!(lab.agents.touched.lock().unwrap().is_empty());
    assert!(lab.agents.hosts.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_delete_of_an_agent_removes_its_app_and_drops_its_stream() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let AdminLab { lab, registry } = admin_lab(permissions.clone()).await;
    let id = lab.create().await.unwrap().app.id().clone();

    AppsRpc::delete_app(&lab.app.scoped(&alice()), id.clone())
        .await
        .unwrap();

    assert!(lab.apps.rows.lock().unwrap().is_empty());
    assert_eq!(registry.disconnected(), std::slice::from_ref(&id));
    assert_eq!(registry.wakes(), [None]);
    assert_eq!(permissions.permissions()[1], Permission::DeleteApp(id));
}

fn agent_id() -> AppId {
    AppId::new("agent-1")
}

struct Agent1 {
    frames: mpsc::Sender<AgentUp>,
    orders: Streaming<AgentDown>,
}

impl Agent1 {
    async fn send(&self, payload: agent_up::Payload) {
        self.frames
            .send(AgentUp {
                payload: Some(payload),
            })
            .await
            .unwrap();
    }

    async fn hello(&self, running: &[&Job]) {
        self.send(agent_up::Payload::Hello(AgentHello {
            running_jobs: running
                .iter()
                .map(|job| common::JobId {
                    value: job.id().to_string(),
                })
                .collect(),
            ..AgentHello::default()
        }))
        .await;
    }

    async fn status(&self, job: &Job, event: Event) {
        self.send(agent_up::Payload::Status(JobStatus {
            job_id: Some(common::JobId {
                value: job.id().to_string(),
            }),
            timestamp: None,
            event: Some(event),
        }))
        .await;
    }

    async fn order(&mut self) -> Option<agent_down::Payload> {
        tokio::time::timeout(Duration::from_secs(5), self.orders.message())
            .await
            .unwrap()
            .unwrap()
            .map(|down| down.payload.unwrap())
    }
}

async fn serve(app: Arc<TestCe>) -> Channel {
    serve_with(app, &InFlight::default()).await
}

async fn serve_with(app: Arc<TestCe>, in_flight: &InFlight) -> Channel {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let grpc = Grpc::from_arc(app).with_in_flight(in_flight);
    let router = Server::builder()
        .add_service(
            AgentServiceServer::new(grpc.clone())
                .max_decoding_message_size(scylla_proto::agent::MAX_MESSAGE_BYTES)
                .max_encoding_message_size(scylla_proto::agent::MAX_MESSAGE_BYTES),
        )
        .add_service(AgentAdminServiceServer::new(grpc));
    tokio::spawn(router.serve_with_incoming(TcpIncoming::from(listener)));
    Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap()
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

async fn open_as(channel: &Channel, token: Option<&str>) -> Result<Agent1, tonic::Status> {
    let (frames, outgoing) = mpsc::channel(16);
    let mut client = AgentServiceClient::new(channel.clone());
    let orders = client
        .open(bearer(ReceiverStream::new(outgoing), token))
        .await?
        .into_inner();
    Ok(Agent1 { frames, orders })
}

async fn open(channel: &Channel) -> Agent1 {
    open_as(channel, Some("app:agent-1")).await.unwrap()
}

async fn until(check: impl Fn() -> bool) {
    for _ in 0..200 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the condition never held");
}

fn cancelled(order: Option<agent_down::Payload>) -> String {
    match order {
        Some(agent_down::Payload::Cancel(PbCancel { job_id: Some(id) })) => id.value,
        other => panic!("expected a cancel, got {other:?}"),
    }
}

fn running_on_agent(pipeline: &scylla_domain::domain::pipeline::Pipeline) -> Job {
    JobBuilder::new(pipeline)
        .running(true)
        .agent(agent_id())
        .build()
}

#[tokio::test]
async fn a_stream_reconciles_its_jobs_takes_a_cancel_and_ends_on_a_disconnect() {
    let pipeline = PipelineBuilder::for_project_id(ProjectId::new("p")).build();
    let (kept, lost) = (running_on_agent(&pipeline), running_on_agent(&pipeline));
    let (lab, registry) = stream_lab(vec![kept.clone(), lost.clone()]).await;
    let channel = serve(lab.app.clone()).await;
    let mut agent = open(&channel).await;

    agent.hello(&[&kept]).await;
    until(|| lab.jobs.row(lost.id()).status() == Phase::Orphaned).await;
    assert_eq!(lab.jobs.row(kept.id()).status(), Phase::Running);
    JobControlRpc::cancel_job(&lab.app.scoped(&alice()), kept.id().clone())
        .await
        .unwrap();
    assert_eq!(cancelled(agent.order().await), lost.id().to_string());
    assert_eq!(cancelled(agent.order().await), kept.id().to_string());

    agent
        .status(&kept, Event::JobCompleted(JobCompleted {}))
        .await;
    registry.disconnect(&agent_id());

    assert!(agent.order().await.is_none());
    assert_eq!(lab.jobs.row(kept.id()).status(), Phase::Cancelled);
    assert!(lab.lines.lines().is_empty());
}

async fn dispatch_pass(lab: &Lab) -> Vec<Job> {
    JobControlRpc::dispatch_pending_jobs(
        &lab.app.scoped(&ctx(CallerContext::Service(
            ServiceIdentity::job_dispatcher(),
        ))),
        None,
    )
    .await
    .unwrap()
}

fn dispatched(order: Option<agent_down::Payload>) -> String {
    match order {
        Some(agent_down::Payload::Dispatch(d)) => d.job_id.unwrap().value,
        other => panic!("expected a dispatch, got {other:?}"),
    }
}

#[tokio::test]
async fn a_job_placed_on_a_stream_stays_through_the_hello_and_returns_when_it_ends() {
    let pipeline = PipelineBuilder::for_project_id(ProjectId::new("p")).build();
    let waiting = JobBuilder::new(&pipeline).build();
    let (lab, registry) = stream_lab(vec![waiting.clone()]).await;
    let channel = serve(lab.app.clone()).await;
    let mut agent = open(&channel).await;

    let placed = dispatch_pass(&lab).await;
    assert_eq!(dispatched(agent.order().await), waiting.id().to_string());
    agent.hello(&[]).await;
    until(|| lab.agents.touches() == 2).await;
    let quiet = tokio::time::timeout(Duration::from_millis(200), agent.orders.message());
    assert!(quiet.await.is_err(), "the hello sends no order");
    assert_eq!(placed.len(), 1);
    assert_eq!(lab.jobs.row(waiting.id()).agent_app_id(), Some(&agent_id()));

    registry.disconnect(&agent_id());

    assert!(agent.order().await.is_none());
    until(|| lab.jobs.row(waiting.id()).agent_app_id().is_none()).await;
    assert_eq!(lab.jobs.row(waiting.id()).status(), Phase::Pending);
}

fn log_line(job: &Job, text: &str) -> agent_up::Payload {
    agent_up::Payload::Log(JobLogLine {
        job_id: Some(common::JobId {
            value: job.id().to_string(),
        }),
        node_id: Some(common::NodeId {
            value: "build".into(),
        }),
        stream: 0,
        line: text.into(),
        timestamp: None,
    })
}

#[tokio::test]
async fn an_agent_gets_a_dispatched_job_sends_its_logs_and_finishes_it() {
    let pipeline = PipelineBuilder::for_project_id(ProjectId::new("p"))
        .nodes(vec![node("build", &[])])
        .build();
    let waiting = JobBuilder::new(&pipeline).build();
    let (lab, _registry) = stream_lab(vec![waiting.clone()]).await;
    let channel = serve(lab.app.clone()).await;
    let mut agent = open(&channel).await;

    dispatch_pass(&lab).await;
    assert_eq!(dispatched(agent.order().await), waiting.id().to_string());
    agent
        .status(&waiting, Event::JobStarted(JobStarted {}))
        .await;
    agent.send(log_line(&waiting, "hello")).await;
    agent.send(log_line(&waiting, "world")).await;
    agent
        .status(&waiting, Event::JobCompleted(JobCompleted {}))
        .await;

    until(|| lab.jobs.row(waiting.id()).status() == Phase::Completed).await;
    assert_eq!(lab.lines.lines(), ["hello", "world"]);
    let published: Vec<String> = lab
        .tails
        .published()
        .iter()
        .map(|l| l.line().to_string())
        .collect();
    assert_eq!(published, ["hello", "world"]);
    assert_eq!(lab.tails.opened(), [waiting.id().clone()]);
    assert_eq!(lab.tails.closed(), [waiting.id().clone()]);
    assert_eq!(lab.agents.touches(), 3);
}

#[tokio::test]
async fn grpc_open_refuses_a_caller_without_a_token_and_a_user() {
    let (lab, _registry) = stream_lab(Vec::new()).await;
    let channel = serve(lab.app.clone()).await;

    let anonymous = open_as(&channel, None).await.err().unwrap();
    assert_eq!(anonymous.code(), Code::Unauthenticated, "{anonymous:?}");
    assert_eq!(
        anonymous.message(),
        "Missing or invalid authorization token"
    );
    let invalid = open_as(&channel, Some("nope")).await.err().unwrap();
    assert_eq!(invalid.code(), Code::Unauthenticated, "{invalid:?}");
    assert_eq!(invalid.message(), "Invalid or expired token");
    let user = open_as(&channel, Some("user:alice")).await.err().unwrap();
    assert_eq!(user.code(), Code::PermissionDenied, "{user:?}");
    assert_eq!(lab.agents.touches(), 0);
}

#[tokio::test]
async fn grpc_a_second_stream_ends_the_first_with_already_exists() {
    let (lab, _registry) = stream_lab(Vec::new()).await;
    let channel = serve(lab.app.clone()).await;
    let mut first = open(&channel).await;
    let _second = open(&channel).await;

    let err = tokio::time::timeout(Duration::from_secs(5), first.orders.message())
        .await
        .unwrap()
        .unwrap_err();

    assert_eq!(err.code(), Code::AlreadyExists);
    assert_eq!(
        err.message(),
        "another agent process opened a stream with this app"
    );
}

#[tokio::test]
async fn grpc_admin_creates_an_agent_and_refuses_before_it_parses() {
    let AdminLab { lab, .. } = admin_lab(Arc::new(RecordingPermissionService::new())).await;
    let channel = serve(lab.app.clone()).await;
    let mut client = AgentAdminServiceClient::new(channel);

    let created = client
        .create_agent(bearer(
            CreateAgentRequest {
                organization_id: Some(common::OrganizationId {
                    value: org().to_string(),
                }),
                name: "runner-1".into(),
                secret_label: None,
            },
            Some("user:alice"),
        ))
        .await
        .unwrap()
        .into_inner();
    let id = created.agent.unwrap().agent_id.unwrap().value;
    assert!(lab.apps.rows.lock().unwrap().contains_key(&AppId::new(&id)));
    assert!(!created.secret.is_empty());

    let anonymous = client
        .get_agent(bearer(GetAgentRequest { agent_id: None }, None))
        .await
        .unwrap_err();
    assert_eq!(anonymous.code(), Code::Unauthenticated, "{anonymous:?}");
    assert_eq!(
        anonymous.message(),
        "Missing or invalid authorization token"
    );
}

#[tokio::test]
async fn grpc_admin_refuses_a_denied_caller() {
    let AdminLab { lab, registry } = admin_lab(Arc::new(DenyingPermissionService::new())).await;
    let channel = serve(lab.app.clone()).await;
    let mut client = AgentAdminServiceClient::new(channel);

    let denied = client
        .delete_agent(bearer(
            DeleteAgentRequest {
                agent_id: Some(common::AppId {
                    value: "agent-1".into(),
                }),
            },
            Some("user:alice"),
        ))
        .await
        .unwrap_err();

    assert_eq!(denied.code(), Code::PermissionDenied, "{denied:?}");
    assert!(registry.disconnected().is_empty());
}

#[tokio::test]
async fn grpc_open_streams_hold_no_permit_of_the_in_flight_budget() {
    let (lab, _registry) = stream_lab(Vec::new()).await;
    let channel = serve_with(lab.app.clone(), &InFlight::waiting(2)).await;
    let mut streams = Vec::new();
    for agent in ["app:agent-1", "app:agent-2", "app:agent-3"] {
        streams.push(open_as(&channel, Some(agent)).await.unwrap());
    }
    let mut client = AgentAdminServiceClient::new(channel);

    let answered = tokio::time::timeout(
        Duration::from_secs(5),
        client.get_agent(bearer(
            GetAgentRequest {
                agent_id: Some(common::AppId {
                    value: "ghost".into(),
                }),
            },
            Some("user:alice"),
        )),
    )
    .await
    .expect("a unary call answers while the streams are open")
    .unwrap_err();

    assert_eq!(answered.code(), Code::NotFound);
    assert_eq!(streams.len(), 3);
}

#[tokio::test]
async fn a_denied_list_get_or_stats_is_forbidden() {
    let AdminLab { lab, .. } = admin_lab(Arc::new(DenyingPermissionService::new())).await;
    let caller = alice();
    let alice = lab.app.scoped(&caller);
    let id = AppId::new("agent-1");

    let list = AgentsRpc::list_agents(&alice, org()).await;
    let get = AgentsRpc::get_agent(&alice, id.clone()).await;
    let stats = AgentsRpc::get_agent_stats(&alice, id).await;

    assert!(matches!(list, Err(DomainError::Forbidden(_))));
    assert!(matches!(get, Err(DomainError::Forbidden(_))));
    assert!(matches!(stats, Err(DomainError::Forbidden(_))));
}
