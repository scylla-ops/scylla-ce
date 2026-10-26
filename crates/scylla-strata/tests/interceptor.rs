//! Tests the strata services behind the CE `AuthInterceptor`, over a real tonic server: the
//! interceptor reads the bearer token one time, and the edition reads the caller from it.

use async_trait::async_trait;
use scylla_auth::authz::Grant;
use scylla_core::application::agent::AgentDispatch;
use scylla_core::application::pagination::{PaginatedResult, PaginationParams};
use scylla_core::application::user::reset::ResetLinks;
use scylla_core::application::user::{AccountRepository, PasswordResetSender, UserRepository};
use scylla_core::application::{
    AgentRepository, AgentStats, AppCredentialRepository, AppRepository, AppTokenRepository,
    HashService, JobLogRepository, JobRepository, PrincipalAccess, SecretResolver,
    SessionRepository,
};
use scylla_core::grpc::auth_interceptor::AuthInterceptor;
use scylla_core::infrastructure::{
    InMemoryAgentRegistry, InMemoryJobLogStream, LogPasswordResetSender,
};
use scylla_core::test_support::authz::RecordingPermissionService;
use scylla_core::test_support::jobs::JobBuilder;
use scylla_core::test_support::pipelines::{PipelineBuilder, node};
use scylla_core::test_support::sessions::SessionBuilder;
use scylla_core::test_support::stubs::{
    EchoResolver, ScopesByAgent, StubAccounts, StubHash, StubJobs, StubRegistry, StubSessions,
    StubTails, StubUsers, dispatcher, plain_hash,
};
use scylla_core::test_support::users::UserBuilder;
use scylla_domain::domain::agent::{Agent, AgentHost};
use scylla_domain::domain::app::{App, AppCredential, AppSecretLabel, AppToken};
use scylla_domain::domain::clock;
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{
    AppCredentialId, AppId, JobId, OrganizationId, ProjectId, SessionId, UserId,
};
use scylla_domain::domain::job::{Job, JobLog};
use scylla_domain::domain::pipeline::NodeId;
use scylla_domain::domain::session::{ACTIVITY_INTERVAL, Session};
use scylla_proto::agent::v1::AgentUp;
use scylla_proto::auth::v1::{LoginRequest, ValidateTokenRequest};
use scylla_proto::common::v1::JobId as PbJobId;
use scylla_proto::job::v1::{GetJobRequest, TailJobLogsRequest};
use scylla_strata::grpc::{
    AgentServiceClient, AgentServiceServer, AuthServiceClient, AuthServiceServer, JobServiceClient,
    JobServiceServer,
};
use scylla_strata::{
    AccessModel, AgentStreams, CedarPermissions, JobDispatch, JobLogTails, ResetLinkBase,
    ScyllaSessions, SessionStore, TrustedForwarding, anonymous,
};
use sqlx::types::chrono::{DateTime, Utc};
use std::sync::{Arc, Mutex};
use strata::{Config, Edition};
use strata_tonic::Grpc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::service::Routes;
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Channel, Server};
use tonic_async_interceptor::async_interceptor;
use tower::ServiceBuilder;

type Duration = <DateTime<Utc> as std::ops::Sub>::Output;

const PASSWORD: &str = "SecurePass123!";
const USER_TOKEN: &str = "user-token";
const APP_TOKEN: &str = "app-token";

#[strata::edition(modules(
    scylla_strata::Access,
    scylla_strata::Apps,
    scylla_strata::Agents,
    scylla_strata::JobControl,
    scylla_strata::Auth
))]
pub struct TestBase;

#[strata::edition(extends = TestBase, overrides(scylla_strata::Access))]
pub struct TestCe;

#[strata::overrides]
impl scylla_strata::Access for TestCe {
    type Authenticator = ScyllaSessions;
    type Permissions = CedarPermissions;
    type Forwarding = TrustedForwarding;
}

/// Counts each token lookup and each activity write of the sessions.
#[derive(Default)]
struct CountingSessions {
    inner: StubSessions,
    finds: Mutex<Vec<String>>,
    touches: Mutex<usize>,
}

impl CountingSessions {
    fn finds_of(&self, token: &str) -> usize {
        self.finds
            .lock()
            .unwrap()
            .iter()
            .filter(|t| *t == token)
            .count()
    }

    fn touches(&self) -> usize {
        *self.touches.lock().unwrap()
    }
}

#[async_trait]
impl SessionRepository for CountingSessions {
    async fn create(&self, session: &Session) -> DomainResult<Session> {
        self.inner.create(session).await
    }
    async fn find_by_token(&self, token: &str) -> DomainResult<Session> {
        self.finds.lock().unwrap().push(token.to_owned());
        self.inner.find_by_token(token).await
    }
    async fn delete_by_token(&self, token: &str) -> DomainResult<()> {
        self.inner.delete_by_token(token).await
    }
    async fn delete_expired(&self) -> DomainResult<u64> {
        self.inner.delete_expired().await
    }
    async fn touch(&self, id: &SessionId, at: DateTime<Utc>) -> DomainResult<bool> {
        *self.touches.lock().unwrap() += 1;
        self.inner.touch(id, at).await
    }
}

/// One live app token. Counts each lookup.
struct CountingTokens {
    token: AppToken,
    finds: Mutex<Vec<String>>,
}

impl CountingTokens {
    fn finds(&self) -> usize {
        self.finds.lock().unwrap().len()
    }
}

#[async_trait]
impl AppTokenRepository for CountingTokens {
    async fn create(&self, _: &AppToken) -> DomainResult<()> {
        unreachable!("no token is issued")
    }
    async fn find_by_token(&self, token: &str) -> DomainResult<AppToken> {
        self.finds.lock().unwrap().push(token.to_owned());
        if token == self.token.token() {
            Ok(self.token.clone())
        } else {
            Err(DomainError::not_found("AppToken", token))
        }
    }
}

struct NoApps;

#[async_trait]
impl AppRepository for NoApps {
    async fn create_app(&self, _: &App, _: &AppCredential) -> DomainResult<()> {
        unreachable!("no app write")
    }
    async fn provision_agent(
        &self,
        _: &App,
        _: &AppCredential,
        _: &Agent,
        _: &Grant,
    ) -> DomainResult<()> {
        unreachable!("no app write")
    }
    async fn provision(&self, _: &App, _: &Grant) -> DomainResult<()> {
        unreachable!("no app write")
    }
    async fn find_by_id(&self, id: &AppId) -> DomainResult<App> {
        Err(DomainError::not_found("App", id))
    }
    async fn find_trigger_runner(&self, _: &OrganizationId) -> DomainResult<Option<AppId>> {
        Ok(None)
    }
    async fn list_by_organization(&self, _: &OrganizationId) -> DomainResult<Vec<App>> {
        Ok(Vec::new())
    }
    async fn set_active(&self, _: &AppId, _: bool) -> DomainResult<()> {
        unreachable!("no app write")
    }
    async fn delete(&self, _: &AppId) -> DomainResult<()> {
        unreachable!("no app write")
    }
    async fn list_access(&self, _: &AppId) -> DomainResult<Vec<PrincipalAccess>> {
        Ok(Vec::new())
    }
}

struct NoCredentials;

#[async_trait]
impl AppCredentialRepository for NoCredentials {
    async fn create(&self, _: &AppCredential) -> DomainResult<()> {
        unreachable!("no secret")
    }
    async fn find_by_id(&self, id: &AppCredentialId) -> DomainResult<AppCredential> {
        Err(DomainError::not_found("AppCredential", id))
    }
    async fn list_by_app(&self, _: &AppId) -> DomainResult<Vec<AppCredential>> {
        Ok(Vec::new())
    }
    async fn list_enabled_by_app(&self, _: &AppId) -> DomainResult<Vec<AppCredential>> {
        Ok(Vec::new())
    }
    async fn set_enabled(&self, _: &AppCredentialId, _: bool) -> DomainResult<()> {
        unreachable!("no secret")
    }
    async fn set_label(&self, _: &AppCredentialId, _: &AppSecretLabel) -> DomainResult<()> {
        unreachable!("no secret")
    }
    async fn delete(&self, _: &AppCredentialId) -> DomainResult<()> {
        unreachable!("no secret")
    }
}

struct QuietAgents;

#[async_trait]
impl AgentRepository for QuietAgents {
    async fn find_by_app_id(&self, app_id: &AppId) -> DomainResult<Agent> {
        Err(DomainError::not_found("Agent", app_id))
    }
    async fn list_by_organization(&self, _: &OrganizationId) -> DomainResult<Vec<Agent>> {
        Ok(Vec::new())
    }
    async fn touch_last_seen(&self, _: &AppId, _: DateTime<Utc>) -> DomainResult<()> {
        Ok(())
    }
    async fn record_host(&self, _: &AppId, _: &AgentHost) -> DomainResult<()> {
        Ok(())
    }
    async fn agent_stats(&self, _: &AppId) -> DomainResult<AgentStats> {
        Ok(AgentStats::default())
    }
}

struct NoLogs;

#[async_trait]
impl JobLogRepository for NoLogs {
    async fn create_many(&self, _: &[JobLog]) -> DomainResult<()> {
        Ok(())
    }
    async fn list_by_job(
        &self,
        _: &JobId,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<JobLog>> {
        Ok(PaginatedResult::new(
            Vec::new(),
            &PaginationParams::default(),
            0,
        ))
    }
    async fn list_by_job_and_node(
        &self,
        _: &JobId,
        _: &NodeId,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<JobLog>> {
        Ok(PaginatedResult::new(
            Vec::new(),
            &PaginationParams::default(),
            0,
        ))
    }
    async fn list_all_by_job(&self, _: &JobId, _: Option<&NodeId>) -> DomainResult<Vec<JobLog>> {
        Ok(Vec::new())
    }
}

struct Lab {
    channel: Channel,
    sessions: Arc<CountingSessions>,
    tokens: Arc<CountingTokens>,
    job: Job,
}

/// A live session of kevin whose activity is due, so that each check writes it.
fn idle_session() -> Session {
    let now = clock::now();
    SessionBuilder::new(&UserId::new("kevin"))
        .token(USER_TOKEN)
        .last_active_at(now - ACTIVITY_INTERVAL - Duration::seconds(1))
        .expires_at(now + Duration::hours(1))
        .build()
}

async fn lab() -> Lab {
    let sessions = Arc::new(CountingSessions::default());
    sessions.inner.create(&idle_session()).await.unwrap();
    let tokens = Arc::new(CountingTokens {
        token: AppToken::create(
            AppId::new("agent-1"),
            AppCredentialId::generate(),
            APP_TOKEN.to_owned(),
            Duration::hours(1),
        ),
        finds: Mutex::new(Vec::new()),
    });
    let users = Arc::new(StubUsers::default());
    users.insert(
        UserBuilder::new("kevin")
            .id(UserId::new("kevin"))
            .password_hash(plain_hash(PASSWORD).as_str())
            .build(),
    );
    let pipeline = PipelineBuilder::for_project_id(ProjectId::new("p"))
        .nodes(vec![node("build", &[])])
        .build();
    let job = JobBuilder::new(&pipeline).build();
    let jobs = Arc::new(StubJobs::with(vec![job.clone()]));
    let registry = Arc::new(StubRegistry::default());
    let tails = Arc::new(StubTails::default());
    let app = TestCe::builder(&Config::new())
        .provide(AccessModel {
            permissions: Arc::new(RecordingPermissionService::new()),
            visibility: Arc::new(ScopesByAgent::everything()),
        })
        .provide(SessionStore {
            sessions: sessions.clone(),
            app_tokens: tokens.clone(),
            trust_forwarded_headers: true,
        })
        .provide(JobDispatch(dispatcher(
            registry.clone(),
            jobs.clone(),
            tails,
        )))
        .provide(JobLogTails(Arc::new(InMemoryJobLogStream::new())))
        .provide(AgentStreams(Arc::new(InMemoryAgentRegistry::new(
            mpsc::unbounded_channel().0,
        ))))
        .provide(ResetLinkBase(ResetLinks::new(None)))
        .provide_arc::<dyn AgentDispatch>(registry)
        .provide_arc::<dyn SecretResolver>(Arc::new(EchoResolver))
        .provide_arc::<dyn JobRepository>(jobs)
        .provide_arc::<dyn JobLogRepository>(Arc::new(NoLogs))
        .provide_arc::<dyn AppRepository>(Arc::new(NoApps))
        .provide_arc::<dyn AppCredentialRepository>(Arc::new(NoCredentials))
        .provide_arc::<dyn AppTokenRepository>(tokens.clone())
        .provide_arc::<dyn HashService>(Arc::new(StubHash::plain()))
        .provide_arc::<dyn AgentRepository>(Arc::new(QuietAgents))
        .provide_arc::<dyn UserRepository>(users.clone())
        .provide_arc::<dyn SessionRepository>(sessions.clone())
        .provide_arc::<dyn AccountRepository>(Arc::new(StubAccounts::new(users)))
        .provide_arc::<dyn PasswordResetSender>(Arc::new(LogPasswordResetSender))
        .build()
        .await
        .unwrap();

    let grpc = Grpc::from_arc(Arc::new(app));
    let auth = async_interceptor(AuthInterceptor::new(sessions.clone(), tokens.clone()));
    let mut routes = Routes::builder();
    routes
        .add_service(
            ServiceBuilder::new()
                .layer(auth.clone())
                .service(JobServiceServer::new(grpc.clone())),
        )
        .add_service(
            ServiceBuilder::new()
                .layer(auth)
                .service(AgentServiceServer::new(grpc.clone())),
        )
        .add_service(AuthServiceServer::with_interceptor(grpc, anonymous));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(
        Server::builder()
            .add_routes(routes.routes())
            .serve_with_incoming(TcpIncoming::from(listener)),
    );
    let channel = Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    Lab {
        channel,
        sessions,
        tokens,
        job,
    }
}

fn bearer<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    request
}

fn job_id(job: &Job) -> Option<PbJobId> {
    Some(PbJobId {
        value: job.id().to_string(),
    })
}

fn login() -> LoginRequest {
    LoginRequest {
        identifier: "kevin".into(),
        password: PASSWORD.into(),
    }
}

#[tokio::test]
async fn a_tail_reads_the_user_token_one_time() {
    let lab = lab().await;
    let mut client = JobServiceClient::new(lab.channel.clone());

    client
        .tail_job_logs(bearer(
            TailJobLogsRequest {
                job_id: job_id(&lab.job),
                node_id: None,
            },
            USER_TOKEN,
        ))
        .await
        .unwrap();

    assert_eq!(lab.sessions.finds_of(USER_TOKEN), 1);
    assert_eq!(lab.sessions.touches(), 1);
    assert_eq!(lab.tokens.finds(), 0);
}

#[tokio::test]
async fn an_agent_stream_reads_the_app_token_one_time() {
    let lab = lab().await;
    let mut client = AgentServiceClient::new(lab.channel.clone());
    let (_frames, outgoing) = mpsc::channel::<AgentUp>(1);

    client
        .open(bearer(ReceiverStream::new(outgoing), APP_TOKEN))
        .await
        .unwrap();

    assert_eq!(lab.tokens.finds(), 1);
    assert_eq!(lab.sessions.finds_of(APP_TOKEN), 1);
}

#[tokio::test]
async fn a_unary_call_reads_the_user_token_one_time() {
    let lab = lab().await;
    let mut client = JobServiceClient::new(lab.channel.clone());

    client
        .get_job(bearer(
            GetJobRequest {
                job_id: job_id(&lab.job),
            },
            USER_TOKEN,
        ))
        .await
        .unwrap();

    assert_eq!(lab.sessions.finds_of(USER_TOKEN), 1);
    assert_eq!(lab.sessions.touches(), 1);
    assert_eq!(lab.tokens.finds(), 0);
}

#[tokio::test]
async fn a_public_auth_call_never_reads_or_touches_its_bearer_token() {
    let lab = lab().await;
    let mut client = AuthServiceClient::new(lab.channel.clone());

    client.login(bearer(login(), USER_TOKEN)).await.unwrap();
    let valid = client
        .validate_token(bearer(
            ValidateTokenRequest {
                token: "another-token".into(),
            },
            USER_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();

    assert!(valid.result.is_some());
    assert_eq!(lab.sessions.finds_of(USER_TOKEN), 0);
    assert_eq!(lab.sessions.finds_of("another-token"), 1);
    assert_eq!(lab.sessions.touches(), 0);
    assert_eq!(lab.tokens.finds(), 0);
}

#[tokio::test]
async fn a_login_behind_a_trusted_proxy_keeps_the_forwarded_address() {
    let lab = lab().await;
    let mut client = AuthServiceClient::new(lab.channel.clone());
    let mut request = tonic::Request::new(login());
    request
        .metadata_mut()
        .insert("x-forwarded-for", "203.0.113.7, 10.0.0.1".parse().unwrap());

    let answer = client.login(request).await.unwrap().into_inner();

    let row = lab
        .sessions
        .inner
        .rows()
        .into_iter()
        .find(|s| s.token() == answer.token)
        .unwrap();
    assert_eq!(
        row.client().ip_address(),
        Some("203.0.113.7".parse().unwrap())
    );
}
