use crate::access::{AccessModel, SessionStore};
use crate::ports::{JobDispatch, ScyllaRepos};
use crate::{
    Access, Agents, Apps, Auth, Grants, JobControl, Organizations, Pipelines, Projects, Roles,
    Secrets, Triggers, Users,
};
use scylla_auth::authz::{PermissionService, VisibilityResolver};
use scylla_core::application::{
    AgentDispatch, AppCredentialRepository, AppRepository, HashService, PipelineRepository,
    SecretResolver,
};
use scylla_core::application::{
    AppTokenRepository, DispatchUseCases, JobRepository, ProjectRepository, SecretCipher,
    SecretRepository, SessionRepository, user::UserRepository,
};
use scylla_core::domain::caller::CallerContext;
use scylla_core::grpc::auth_interceptor::{AuthContext, AuthInterceptor};
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use strata::InFlight;
use strata::{Builder, Edition, Resource};
use strata_tonic::{Grpc, OnDeadline};
use tonic::service::RoutesBuilder;
use tower::ServiceBuilder;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slice {
    Projects,
    Secrets,
    Pipelines,
    Apps,
    Roles,
    Grants,
    Organizations,
    Users,
    Auth,
    Triggers,
    Jobs,
    Agents,
}

impl Slice {
    pub const ALL: &'static [Slice] = &[
        Slice::Projects,
        Slice::Secrets,
        Slice::Pipelines,
        Slice::Apps,
        Slice::Roles,
        Slice::Grants,
        Slice::Organizations,
        Slice::Users,
        Slice::Auth,
        Slice::Triggers,
        Slice::Jobs,
        Slice::Agents,
    ];

    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Slice::Projects => "projects",
            Slice::Secrets => "secrets",
            Slice::Pipelines => "pipelines",
            Slice::Apps => "apps",
            Slice::Roles => "roles",
            Slice::Grants => "grants",
            Slice::Organizations => "organizations",
            Slice::Users => "users",
            Slice::Auth => "auth",
            Slice::Triggers => "triggers",
            Slice::Jobs => "jobs",
            Slice::Agents => "agents",
        }
    }

    #[must_use]
    pub fn parse(key: &str) -> Option<Slice> {
        Slice::ALL.iter().copied().find(|s| s.key() == key)
    }
}

impl fmt::Display for Slice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SwitchError {
    Unknown(String),
    NotReady(Slice),
}

impl fmt::Display for SwitchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SwitchError::Unknown(key) => write!(f, "unknown strata slice {key:?}"),
            SwitchError::NotReady(slice) => write!(
                f,
                "the edition does not mark the slice {slice:?} ready for strata"
            ),
        }
    }
}

impl std::error::Error for SwitchError {}

pub fn switched(keys: &[String], ready: &[Slice]) -> Result<Vec<Slice>, SwitchError> {
    let mut slices = Vec::new();
    for key in keys {
        let slice = Slice::parse(key).ok_or_else(|| SwitchError::Unknown(key.clone()))?;
        if !ready.contains(&slice) {
            return Err(SwitchError::NotReady(slice));
        }
        if !slices.contains(&slice) {
            slices.push(slice);
        }
    }
    Ok(slices)
}

#[derive(Clone)]
pub struct Shared {
    pub db: sqlx::PgPool,
    pub permissions: Arc<dyn PermissionService>,
    pub visibility: Arc<dyn VisibilityResolver>,
    pub sessions: Arc<dyn SessionRepository>,
    pub app_tokens: Arc<dyn AppTokenRepository>,
    pub projects: Arc<dyn ProjectRepository>,
    pub users: Arc<dyn UserRepository>,
    pub jobs: Arc<dyn JobRepository>,
    pub dispatch: Arc<DispatchUseCases>,
    pub secrets: Arc<dyn SecretRepository>,
    pub cipher: Arc<dyn SecretCipher>,
    pub pipelines: Arc<dyn PipelineRepository>,
    pub secret_resolver: Arc<dyn SecretResolver>,
    pub apps: Arc<dyn AppRepository>,
    pub app_credentials: Arc<dyn AppCredentialRepository>,
    pub hash: Arc<dyn HashService>,
    pub agent_registry: Arc<dyn AgentDispatch>,
    pub roles: Arc<dyn scylla_auth::authz::RoleRepository>,
    pub grants: Arc<dyn scylla_auth::authz::GrantRepository>,
    pub authz_entities: Arc<dyn scylla_auth::authz::AuthzEntityProvider>,
    pub organizations: Arc<dyn scylla_core::application::OrganizationRepository>,
    pub accounts: Arc<dyn scylla_core::application::user::AccountRepository>,
    pub reset_sender: Arc<dyn scylla_core::application::PasswordResetSender>,
    pub reset_links: scylla_core::application::user::reset::ResetLinks,
    pub trigger_repo: Arc<dyn scylla_core::application::TriggerRepository>,
    pub cron_schedule: Arc<dyn scylla_core::application::CronSchedule>,
    pub webhook_base_url: Option<String>,
    pub job_logs: Arc<dyn scylla_core::application::JobLogRepository>,
    pub job_log_stream: Arc<scylla_core::infrastructure::InMemoryJobLogStream>,
    pub agents: Arc<dyn scylla_core::application::AgentRepository>,
    pub agent_streams: Arc<scylla_core::infrastructure::InMemoryAgentRegistry>,
    pub trust_forwarded_headers: bool,
}

pub type AuthInterceptorLayer = tonic_async_interceptor::AsyncInterceptorLayer<AuthInterceptor>;

pub type MountFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

pub trait Host: Send + Sync + 'static {
    fn ready(&self) -> &[Slice];

    fn mount<'a>(
        &'a self,
        shared: &'a Shared,
        slices: &'a [Slice],
        routes: &'a mut RoutesBuilder,
        auth: &'a AuthInterceptorLayer,
        in_flight: &'a InFlight,
    ) -> MountFuture<'a>;
}

type Provide<E> = Box<dyn for<'c> Fn(Builder<'c, E>) -> Builder<'c, E> + Send + Sync>;

pub struct EditionHost<E> {
    ready: &'static [Slice],
    provided: Vec<Provide<E>>,
    edition: PhantomData<fn() -> E>,
}

impl<E: Edition> EditionHost<E> {
    #[must_use]
    pub fn new(ready: &'static [Slice]) -> Self {
        Self {
            ready,
            provided: Vec::new(),
            edition: PhantomData,
        }
    }

    #[must_use]
    pub fn provide<R: Resource + Clone + Sync>(mut self, resource: R) -> Self {
        self.provided
            .push(Box::new(move |builder| builder.provide(resource.clone())));
        self
    }
}

impl<E> Host for EditionHost<E>
where
    E: Edition
        + Access
        + Projects
        + Secrets
        + Pipelines
        + Apps
        + Roles
        + Grants
        + Organizations
        + Users
        + Auth
        + Triggers
        + JobControl
        + Agents
        + Send
        + Sync
        + 'static,
    <E as strata::Contextual>::Ctx: crate::CallerOf,
{
    fn ready(&self) -> &[Slice] {
        self.ready
    }

    fn mount<'a>(
        &'a self,
        shared: &'a Shared,
        slices: &'a [Slice],
        routes: &'a mut RoutesBuilder,
        auth: &'a AuthInterceptorLayer,
        in_flight: &'a InFlight,
    ) -> MountFuture<'a> {
        Box::pin(async move {
            if slices.is_empty() {
                return Ok(());
            }
            let app = build::<E>(shared, &self.provided).await?;
            let grpc = Grpc::from_arc(app)
                .with_in_flight(in_flight)
                .on_deadline(OnDeadline::Cancel)
                .rejection_prefix(false)
                .echo_request_id(false);
            macro_rules! authed {
                ($service:expr $(,)?) => {
                    routes.add_service(ServiceBuilder::new().layer(auth.clone()).service($service))
                };
            }
            for slice in slices {
                match slice {
                    Slice::Projects => {
                        authed!(crate::grpc::ProjectServiceServer::new(grpc.clone()));
                    }
                    Slice::Secrets => {
                        authed!(crate::grpc::SecretServiceServer::new(grpc.clone()));
                    }
                    Slice::Pipelines => {
                        authed!(crate::grpc::PipelineServiceServer::new(grpc.clone()),);
                    }
                    Slice::Apps => {
                        authed!(crate::grpc::AppServiceServer::new(grpc.clone()));
                        routes.add_service(crate::grpc::AppAuthServiceServer::with_interceptor(
                            grpc.clone(),
                            anonymous,
                        ));
                    }
                    Slice::Roles => {
                        authed!(crate::grpc::RoleServiceServer::new(grpc.clone()));
                    }
                    Slice::Grants => {
                        authed!(crate::grpc::GrantServiceServer::new(grpc.clone()));
                    }
                    Slice::Organizations => {
                        authed!(crate::grpc::OrganizationServiceServer::new(grpc.clone()),);
                    }
                    Slice::Users => {
                        authed!(crate::grpc::UserServiceServer::new(grpc.clone()));
                    }
                    Slice::Auth => {
                        routes.add_service(crate::grpc::AuthServiceServer::with_interceptor(
                            grpc.clone(),
                            anonymous,
                        ));
                    }
                    Slice::Triggers => {
                        authed!(crate::grpc::TriggerServiceServer::new(grpc.clone()));
                    }
                    Slice::Jobs => {
                        authed!(crate::grpc::JobServiceServer::new(grpc.clone()));
                    }
                    Slice::Agents => {
                        authed!(
                            crate::grpc::AgentServiceServer::new(grpc.clone())
                                .max_decoding_message_size(scylla_proto::agent::MAX_MESSAGE_BYTES)
                                .max_encoding_message_size(scylla_proto::agent::MAX_MESSAGE_BYTES),
                        );
                        authed!(crate::grpc::AgentAdminServiceServer::new(grpc.clone()),);
                    }
                }
                tracing::info!(slice = %slice, edition = std::any::type_name::<E>(), "strata serves the slice");
            }
            Ok(())
        })
    }
}

#[allow(clippy::result_large_err, clippy::unnecessary_wraps)]
fn anonymous(mut request: tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> {
    request
        .extensions_mut()
        .insert(AuthContext::new(CallerContext::Anonymous));
    Ok(request)
}

async fn build<
    E: Edition
        + Access
        + Projects
        + Secrets
        + Pipelines
        + Apps
        + Roles
        + Grants
        + Organizations
        + Users
        + Auth
        + Triggers
        + JobControl
        + Agents,
>(
    shared: &Shared,
    provided: &[Provide<E>],
) -> anyhow::Result<Arc<E>> {
    let config = strata::Config::new();
    let builder = provided
        .iter()
        .fold(E::builder(&config), |builder, provide| provide(builder));
    let app = builder
        .provide(ScyllaRepos {
            db: shared.db.clone(),
            projects: shared.projects.clone(),
            users: shared.users.clone(),
            jobs: shared.jobs.clone(),
        })
        .provide(JobDispatch(shared.dispatch.clone()))
        .provide_arc(shared.secrets.clone())
        .provide_arc(shared.cipher.clone())
        .provide_arc(shared.pipelines.clone())
        .provide_arc(shared.projects.clone())
        .provide_arc(shared.jobs.clone())
        .provide_arc(shared.secret_resolver.clone())
        .provide_arc(shared.apps.clone())
        .provide_arc(shared.app_credentials.clone())
        .provide_arc(shared.app_tokens.clone())
        .provide_arc(shared.hash.clone())
        .provide_arc(shared.agent_registry.clone())
        .provide_arc(shared.roles.clone())
        .provide_arc(shared.grants.clone())
        .provide_arc(shared.authz_entities.clone())
        .provide_arc(shared.organizations.clone())
        .provide_arc(shared.users.clone())
        .provide_arc(shared.accounts.clone())
        .provide_arc(shared.reset_sender.clone())
        .provide(crate::ResetLinkBase(shared.reset_links.clone()))
        .provide_arc(shared.sessions.clone())
        .provide_arc(shared.trigger_repo.clone())
        .provide_arc(shared.cron_schedule.clone())
        .provide(crate::TriggerWebhookBase(shared.webhook_base_url.clone()))
        .provide(crate::JobLogTails(shared.job_log_stream.clone()))
        .provide(crate::AgentStreams(shared.agent_streams.clone()))
        .provide_arc(shared.job_logs.clone())
        .provide_arc(shared.agents.clone())
        .production()
        .provide(AccessModel {
            permissions: shared.permissions.clone(),
            visibility: shared.visibility.clone(),
        })
        .provide(SessionStore {
            sessions: shared.sessions.clone(),
            app_tokens: shared.app_tokens.clone(),
            trust_forwarded_headers: shared.trust_forwarded_headers,
        })
        .build()
        .await
        .map_err(|e| anyhow::anyhow!("strata edition {}: {e}", std::any::type_name::<E>()))?;
    Ok(Arc::new(app))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_switch_needs_a_known_and_ready_slice() {
        let keys = |k: &[&str]| k.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(switched(&keys(&[]), &[]), Ok(vec![]));
        assert_eq!(
            switched(&keys(&["projects", "projects"]), &[Slice::Projects]),
            Ok(vec![Slice::Projects])
        );
        assert_eq!(
            switched(&keys(&["projects"]), &[]),
            Err(SwitchError::NotReady(Slice::Projects))
        );
        assert_eq!(
            switched(&keys(&["nope"]), Slice::ALL),
            Err(SwitchError::Unknown("nope".to_owned()))
        );
    }
}
