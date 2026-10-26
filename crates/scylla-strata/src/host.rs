use crate::access::{AccessModel, SessionStore};
use crate::ports::{JobDispatch, ScyllaRepos};
use crate::{
    Access, Agents, Apps, Auth, Grants, JobControl, Organizations, Pipelines, Projects, Roles,
    Secrets, Triggers, Users,
};
use scylla_auth::authz::{PermissionService, VisibilityResolver};
use scylla_core::application::IngestOutcome;
use scylla_core::application::{
    AgentDispatch, AppCredentialRepository, AppRepository, HashService, PipelineRepository,
    SecretResolver,
};
use scylla_core::application::{
    AppTokenRepository, DispatchUseCases, JobRepository, ProjectRepository, SecretCipher,
    SecretRepository, SessionRepository, user::UserRepository,
};
use scylla_core::config::BootstrapConfig;
use scylla_core::domain::caller::CallerContext;
use scylla_core::domain::errors::DomainResult;
use scylla_core::error::BootstrapError;
use scylla_core::grpc::auth_interceptor::{AuthContext, AuthInterceptor};
use scylla_core::rest::webhook::{WebhookDelivery, WebhookIngress};
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

/// A gRPC service of the CE backend. Each service belongs to one slice (see
/// [`Slice::services`]). The server and the edition use this one table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Service {
    Projects,
    Secrets,
    Pipelines,
    Apps,
    AppAuth,
    Roles,
    Grants,
    Organizations,
    Users,
    Auth,
    Triggers,
    Jobs,
    Agents,
    AgentAdmin,
}

impl Service {
    pub const ALL: &'static [Service] = &[
        Service::Projects,
        Service::Secrets,
        Service::Pipelines,
        Service::Apps,
        Service::AppAuth,
        Service::Roles,
        Service::Grants,
        Service::Organizations,
        Service::Users,
        Service::Auth,
        Service::Triggers,
        Service::Jobs,
        Service::Agents,
        Service::AgentAdmin,
    ];

    /// The full gRPC name of the service.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Service::Projects => "scylla.project.v1.ProjectService",
            Service::Secrets => "scylla.secret.v1.SecretService",
            Service::Pipelines => "scylla.pipeline.v1.PipelineService",
            Service::Apps => "scylla.app.v1.AppService",
            Service::AppAuth => "scylla.app.v1.AppAuthService",
            Service::Roles => "scylla.authz.v1.RoleService",
            Service::Grants => "scylla.authz.v1.GrantService",
            Service::Organizations => "scylla.organization.v1.OrganizationService",
            Service::Users => "scylla.user.v1.UserService",
            Service::Auth => "scylla.auth.v1.AuthService",
            Service::Triggers => "scylla.trigger.v1.TriggerService",
            Service::Jobs => "scylla.job.v1.JobService",
            Service::Agents => "scylla.agent.v1.AgentService",
            Service::AgentAdmin => "scylla.agent.v1.AgentAdminService",
        }
    }

    /// The slice of the service.
    #[must_use]
    pub fn slice(self) -> Slice {
        match self {
            Service::Projects => Slice::Projects,
            Service::Secrets => Slice::Secrets,
            Service::Pipelines => Slice::Pipelines,
            Service::Apps | Service::AppAuth => Slice::Apps,
            Service::Roles => Slice::Roles,
            Service::Grants => Slice::Grants,
            Service::Organizations => Slice::Organizations,
            Service::Users => Slice::Users,
            Service::Auth => Slice::Auth,
            Service::Triggers => Slice::Triggers,
            Service::Jobs => Slice::Jobs,
            Service::Agents | Service::AgentAdmin => Slice::Agents,
        }
    }
}

impl Slice {
    /// The gRPC services of the slice, from the table of [`Service::slice`].
    pub fn services(self) -> impl Iterator<Item = Service> {
        Service::ALL
            .iter()
            .copied()
            .filter(move |s| s.slice() == self)
    }
}

/// Writes one line for each worker and for the webhook ingress: the engine that runs it. The engine
/// is always strata.
pub fn log_engines() {
    for worker in crate::Worker::ALL {
        tracing::info!(target: "scylla_strata", worker = ?worker, engine = "strata", "the engine runs the worker");
    }
    tracing::info!(target: "scylla_strata", engine = "strata", "the engine runs the webhook ingress");
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
    pub roles: Arc<dyn scylla_auth::authz::RoleRepository>,
    pub grants: Arc<dyn scylla_auth::authz::GrantRepository>,
    pub authz_entities: Arc<dyn scylla_auth::authz::AuthzEntityProvider>,
    pub organizations: Arc<dyn scylla_core::application::OrganizationRepository>,
    pub accounts: Arc<dyn scylla_core::application::user::AccountRepository>,
    pub reset_sender: Arc<dyn scylla_core::application::PasswordResetSender>,
    pub reset_links: scylla_core::application::user::reset::ResetLinks,
    pub trigger_repo: Arc<dyn scylla_core::application::TriggerRepository>,
    pub trigger_deliveries: Arc<dyn scylla_core::application::TriggerDeliveryRepository>,
    pub cron_schedule: Arc<dyn scylla_core::application::CronSchedule>,
    pub webhook_base_url: Option<String>,
    pub job_logs: Arc<dyn scylla_core::application::JobLogRepository>,
    pub job_log_stream: Arc<scylla_core::infrastructure::InMemoryJobLogStream>,
    pub agents: Arc<dyn scylla_core::application::AgentRepository>,
    /// The one agent registry: the agent stream registers on it, and the Apps slot is the same
    /// value.
    pub agent_streams: Arc<scylla_core::infrastructure::InMemoryAgentRegistry>,
    pub trust_forwarded_headers: bool,
}

pub type AuthInterceptorLayer = tonic_async_interceptor::AsyncInterceptorLayer<AuthInterceptor>;

pub type BuildFuture<'a> =
    Pin<Box<dyn Future<Output = anyhow::Result<Box<dyn Built>>> + Send + 'a>>;

pub type BootstrapFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), BootstrapError>> + Send + 'a>>;

pub trait Host: Send + Sync + 'static {
    /// The slices that the edition serves. The server refuses to start when a slice of
    /// [`Slice::ALL`] is not in it: no other engine serves the slice.
    fn ready(&self) -> &[Slice];

    /// Builds the production edition on the CE adapters of `shared`. The server builds it one
    /// time, before the bootstrap and the routes.
    fn build<'a>(&'a self, shared: &'a Shared) -> BuildFuture<'a>;
}

/// A built edition. The server serves every slice with it, and runs the bootstrap and the webhook
/// ingress on it.
pub trait Built: Send + Sync {
    /// Serves `slices` on `routes`, and starts the worker of each slice.
    fn mount(
        &self,
        slices: &[Slice],
        routes: &mut RoutesBuilder,
        auth: &AuthInterceptorLayer,
        in_flight: &InFlight,
        workers: crate::Workers,
    );

    /// Runs the CE admin bootstrap on the Users and Grants modules.
    fn bootstrap<'a>(&'a self, cfg: &'a BootstrapConfig) -> BootstrapFuture<'a>;

    /// The webhook ingress of the Triggers module, for the CE webhook route.
    fn webhooks(&self) -> Arc<dyn WebhookIngress>;
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

impl<E: Edition + Send + Sync + 'static> EditionHost<E>
where
    E: Access
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
{
    /// Builds the edition on the CE adapters of `shared`, as `Host::build` does. With
    /// `production` false, a dev component is accepted (for tests).
    pub async fn edition(&self, shared: &Shared, production: bool) -> anyhow::Result<Arc<E>> {
        build::<E>(shared, &self.provided, production).await
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

    fn build<'a>(&'a self, shared: &'a Shared) -> BuildFuture<'a> {
        Box::pin(async move {
            let app = build::<E>(shared, &self.provided, true).await?;
            Ok(Box::new(BuiltEdition(app)) as Box<dyn Built>)
        })
    }
}

struct BuiltEdition<E>(Arc<E>);

struct EditionWebhooks<E>(Arc<E>);

#[async_trait::async_trait]
impl<E> WebhookIngress for EditionWebhooks<E>
where
    E: Triggers + Send + Sync + 'static,
{
    async fn ingest(&self, delivery: WebhookDelivery) -> DomainResult<IngestOutcome> {
        Triggers::ingest_webhook(&*self.0, delivery).await
    }
}

impl<E> Built for BuiltEdition<E>
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
    fn bootstrap<'a>(&'a self, cfg: &'a BootstrapConfig) -> BootstrapFuture<'a> {
        Box::pin(crate::bootstrap_admin(&*self.0, cfg))
    }

    fn webhooks(&self) -> Arc<dyn WebhookIngress> {
        Arc::new(EditionWebhooks(self.0.clone()))
    }

    fn mount(
        &self,
        slices: &[Slice],
        routes: &mut RoutesBuilder,
        auth: &AuthInterceptorLayer,
        in_flight: &InFlight,
        workers: crate::Workers,
    ) {
        let app = self.0.clone();
        crate::workers::spawn(&app, slices, workers);
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
            for service in slice.services() {
                match service {
                    Service::Projects => {
                        authed!(crate::grpc::ProjectServiceServer::new(grpc.clone()));
                    }
                    Service::Secrets => {
                        authed!(crate::grpc::SecretServiceServer::new(grpc.clone()));
                    }
                    Service::Pipelines => {
                        authed!(crate::grpc::PipelineServiceServer::new(grpc.clone()));
                    }
                    Service::Apps => {
                        authed!(crate::grpc::AppServiceServer::new(grpc.clone()));
                    }
                    Service::AppAuth => {
                        routes.add_service(crate::grpc::AppAuthServiceServer::with_interceptor(
                            grpc.clone(),
                            anonymous,
                        ));
                    }
                    Service::Roles => {
                        authed!(crate::grpc::RoleServiceServer::new(grpc.clone()));
                    }
                    Service::Grants => {
                        authed!(crate::grpc::GrantServiceServer::new(grpc.clone()));
                    }
                    Service::Organizations => {
                        authed!(crate::grpc::OrganizationServiceServer::new(grpc.clone()));
                    }
                    Service::Users => {
                        authed!(crate::grpc::UserServiceServer::new(grpc.clone()));
                    }
                    Service::Auth => {
                        routes.add_service(crate::grpc::AuthServiceServer::with_interceptor(
                            grpc.clone(),
                            anonymous,
                        ));
                    }
                    Service::Triggers => {
                        authed!(crate::grpc::TriggerServiceServer::new(grpc.clone()));
                    }
                    Service::Jobs => {
                        authed!(crate::grpc::JobServiceServer::new(grpc.clone()));
                    }
                    Service::Agents => {
                        authed!(
                            crate::grpc::AgentServiceServer::new(grpc.clone())
                                .max_decoding_message_size(scylla_proto::agent::MAX_MESSAGE_BYTES)
                                .max_encoding_message_size(scylla_proto::agent::MAX_MESSAGE_BYTES),
                        );
                    }
                    Service::AgentAdmin => {
                        authed!(crate::grpc::AgentAdminServiceServer::new(grpc.clone()));
                    }
                }
            }
            tracing::info!(slice = %slice, edition = std::any::type_name::<E>(), "strata serves the slice");
        }
    }
}

/// The interceptor of a public service (`AuthService`, `AppAuthService`): it marks the caller
/// anonymous and reads no token, as CE does.
#[allow(clippy::result_large_err, clippy::unnecessary_wraps)]
pub fn anonymous(mut request: tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> {
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
    production: bool,
) -> anyhow::Result<Arc<E>> {
    let config = strata::Config::new();
    let builder = provided
        .iter()
        .fold(E::builder(&config), |builder, provide| provide(builder));
    let builder = builder
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
        .provide_arc::<dyn AgentDispatch>(shared.agent_streams.clone())
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
        .provide_arc(shared.trigger_deliveries.clone())
        .provide_arc(shared.cron_schedule.clone())
        .provide(crate::TriggerWebhookBase(shared.webhook_base_url.clone()))
        .provide(crate::JobLogTails(shared.job_log_stream.clone()))
        .provide(crate::AgentStreams(shared.agent_streams.clone()))
        .provide_arc(shared.job_logs.clone())
        .provide_arc(shared.agents.clone());
    let builder = if production {
        builder.production()
    } else {
        builder
    };
    let app = builder
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
    use crate::grpc::*;
    use tonic::server::NamedService;

    fn name_of<S: NamedService>() -> &'static str {
        S::NAME
    }

    #[test]
    fn the_service_table_names_each_generated_server_once() {
        type G = Grpc<crate::ScyllaCe>;
        let generated = [
            (Service::Projects, name_of::<ProjectServiceServer<G>>()),
            (Service::Secrets, name_of::<SecretServiceServer<G>>()),
            (Service::Pipelines, name_of::<PipelineServiceServer<G>>()),
            (Service::Apps, name_of::<AppServiceServer<G>>()),
            (Service::AppAuth, name_of::<AppAuthServiceServer<G>>()),
            (Service::Roles, name_of::<RoleServiceServer<G>>()),
            (Service::Grants, name_of::<GrantServiceServer<G>>()),
            (
                Service::Organizations,
                name_of::<OrganizationServiceServer<G>>(),
            ),
            (Service::Users, name_of::<UserServiceServer<G>>()),
            (Service::Auth, name_of::<AuthServiceServer<G>>()),
            (Service::Triggers, name_of::<TriggerServiceServer<G>>()),
            (Service::Jobs, name_of::<JobServiceServer<G>>()),
            (Service::Agents, name_of::<AgentServiceServer<G>>()),
            (Service::AgentAdmin, name_of::<AgentAdminServiceServer<G>>()),
        ];
        assert_eq!(generated.len(), Service::ALL.len());
        for (service, name) in generated {
            assert_eq!(service.name(), name);
        }
        let mut seen = Vec::new();
        for slice in Slice::ALL {
            let services: Vec<Service> = slice.services().collect();
            assert!(!services.is_empty(), "{slice} has no service");
            seen.extend(services);
        }
        assert_eq!(seen.len(), Service::ALL.len());
        assert!(Service::ALL.iter().all(|s| seen.contains(s)));
        assert_eq!(
            Slice::Agents.services().collect::<Vec<_>>(),
            [Service::Agents, Service::AgentAdmin]
        );
        assert_eq!(
            Slice::Apps.services().collect::<Vec<_>>(),
            [Service::Apps, Service::AppAuth]
        );
    }
}
