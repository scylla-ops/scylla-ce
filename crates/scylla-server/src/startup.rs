use crate::surface::Surface;
use http::{HeaderName, HeaderValue, Method};
use scylla_auth::audit::AuditLog;
use scylla_auth::cedar::CedarPermissionService;
use scylla_core::application::user::reset::ResetLinks;
use scylla_core::application::{
    AgentUseCases, AppTokenUseCases, AppUseCases, AuthUseCases, BootstrapUseCases, CronSchedule,
    DispatchSecretResolver, DispatchUseCases, GrantUseCases, JobLogUseCases, JobReaper,
    JobUseCases, OrganizationUseCases, PasswordResetSender, PasswordResetUseCases,
    PendingJobScheduler, PermissionAuthorizer, PipelineUseCases, ProjectUseCases, RoleUseCases,
    SecretCipher, SecretResolver, SecretUseCases, SessionSweeper, TriggerCronScheduler,
    TriggerFireUseCases, TriggerFirer, TriggerFiring, TriggerUseCases, UserUseCases,
    WebhookIngressUseCases,
};
use scylla_core::config::ControlPlaneConfig;
use scylla_core::domain::ids::AppId;
use scylla_core::error::StartupError;
use scylla_core::grpc::auth_interceptor::AuthInterceptor;
use scylla_core::infrastructure::{
    Argon2HashService, ChaChaSecretCipher, CronScheduleService, InMemoryAgentRegistry,
    InMemoryJobLogStream,
};
use scylla_db::{
    PgAccountRepository, PgAgentRepository, PgAppCredentialRepository, PgAppRepository,
    PgAppTokenRepository, PgAuditLog, PgAuthzEntityProvider, PgGrantRepository, PgJobLogRepository,
    PgJobRepository, PgOrganizationRepository, PgPipelineRepository, PgProjectRepository,
    PgRoleRepository, PgSecretRepository, PgSessionRepository, PgTriggerDeliveryRepository,
    PgTriggerRepository, PgUserRepository,
};
use scylla_extension::{Actions, Hooks};
use scylla_strata::{BOOTSTRAP_SLICES, Built, Service, Slice, Worker};
use sqlx::PgPool;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tonic_async_interceptor::async_interceptor;
use tower_http::classify::{GrpcCode, GrpcErrorsAsFailures, SharedClassifier};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

pub(crate) struct Services {
    pub shared: scylla_strata::Shared,
    pub auth_uc: Arc<AuthUseCases>,
    pub user_uc: Arc<UserUseCases>,
    pub reset_uc: Arc<PasswordResetUseCases>,
    pub org_uc: Arc<OrganizationUseCases>,
    pub actions: Arc<Actions>,
    pub project_uc: Arc<ProjectUseCases>,
    pub pipeline_uc: Arc<PipelineUseCases>,
    pub trigger_uc: Arc<TriggerUseCases>,
    pub trigger_fire_uc: Arc<TriggerFireUseCases>,
    pub webhook_ingress_uc: Arc<WebhookIngressUseCases>,
    pub secret_uc: Arc<SecretUseCases>,
    pub job_uc: Arc<JobUseCases>,
    pub job_log_uc: Arc<JobLogUseCases>,
    pub app_uc: Arc<AppUseCases>,
    pub app_token_uc: Arc<AppTokenUseCases>,
    pub agent_uc: Arc<AgentUseCases>,
    pub agent_registry: Arc<InMemoryAgentRegistry>,
    pub grant_uc: Arc<GrantUseCases>,
    pub role_uc: Arc<RoleUseCases>,
    pub permission_checker: Arc<CedarPermissionService<PgAuthzEntityProvider>>,
    pub session_repo: Arc<PgSessionRepository>,
    pub app_token_repo: Arc<PgAppTokenRepository>,
    pub wakes: Option<mpsc::UnboundedReceiver<Option<AppId>>>,
}

pub(crate) fn ce_workers(switched: &[Slice]) -> Vec<Worker> {
    Worker::ALL
        .iter()
        .copied()
        .filter(|worker| !worker.on_strata(switched))
        .collect()
}

/// The CE adapters that a strata edition takes (`EditionHost::edition`), on `db`, with no
/// Actions worker started. For the tests of an edition.
pub async fn strata_shared(
    config: &ControlPlaneConfig,
    db: PgPool,
    reset_sender: Arc<dyn PasswordResetSender>,
) -> Result<scylla_strata::Shared, StartupError> {
    let services = init_services(
        config,
        db,
        Arc::new(Hooks::default()),
        reset_sender,
        Slice::ALL,
    )
    .await?;
    bootstrap(config, &services, None, &[]).await?;
    Ok(services.shared)
}

/// Runs the admin bootstrap of `config`. It runs on `built` when each slice of
/// `BOOTSTRAP_SLICES` is in `switched`, and on the CE Actions in the other cases.
pub(crate) async fn bootstrap(
    config: &ControlPlaneConfig,
    services: &Services,
    built: Option<&dyn Built>,
    switched: &[Slice],
) -> Result<(), StartupError> {
    let Some(cfg) = &config.bootstrap else {
        return Ok(());
    };
    match built {
        Some(built) if BOOTSTRAP_SLICES.iter().all(|s| switched.contains(s)) => {
            built.bootstrap(cfg).await?;
        }
        _ => {
            let bootstrap_uc = BootstrapUseCases::new(
                services.actions.clone(),
                services.user_uc.clone(),
                services.grant_uc.clone(),
            );
            scylla_core::bootstrap::bootstrap_admin(&bootstrap_uc, cfg).await?;
        }
    }
    Ok(())
}

/// The webhook route. It runs on `built` when the triggers slice is in `switched`, and on the CE
/// Actions in the other cases.
pub(crate) fn webhook_router(
    services: &Services,
    built: Option<&dyn Built>,
    switched: &[Slice],
) -> axum::Router {
    match built {
        Some(built) if switched.contains(&Slice::Triggers) => {
            scylla_core::rest::webhook::router_on(built.webhooks())
        }
        _ => scylla_core::rest::webhook::router(
            services.actions.clone(),
            services.webhook_ingress_uc.clone(),
        ),
    }
}

pub(crate) async fn init_services(
    config: &ControlPlaneConfig,
    db: PgPool,
    hooks: Arc<Hooks>,
    reset_sender: Arc<dyn PasswordResetSender>,
    switched: &[Slice],
) -> Result<Services, StartupError> {
    let ce = ce_workers(switched);
    let user_repo = Arc::new(PgUserRepository::new(db.clone()));
    let account_repo = Arc::new(PgAccountRepository::new(db.clone()));
    let session_repo = Arc::new(PgSessionRepository::new(db.clone()));
    let org_repo = Arc::new(PgOrganizationRepository::new(db.clone()));
    let project_repo = Arc::new(PgProjectRepository::new(db.clone()));
    let pipeline_repo = Arc::new(PgPipelineRepository::new(db.clone()));
    let secret_repo = Arc::new(PgSecretRepository::new(db.clone()));
    let job_repo = Arc::new(PgJobRepository::new(db.clone()));
    let job_log_repo = Arc::new(PgJobLogRepository::new(db.clone()));
    let app_repo = Arc::new(PgAppRepository::new(db.clone()));
    let app_credential_repo = Arc::new(PgAppCredentialRepository::new(db.clone()));
    let app_token_repo = Arc::new(PgAppTokenRepository::new(db.clone()));
    let agent_repo = Arc::new(PgAgentRepository::new(db.clone()));
    let authz_provider = Arc::new(PgAuthzEntityProvider::new(db.clone()));
    let role_repo = Arc::new(PgRoleRepository::new(db.clone()));
    let grant_repo = Arc::new(PgGrantRepository::new(db.clone()));
    let hash_service = Arc::new(Argon2HashService::new());

    let secret_cipher: Arc<dyn SecretCipher> = Arc::new(ChaChaSecretCipher::from_hex_key(
        config.secrets.as_ref().map(|s| s.master_key.as_str()),
    )?);
    let secret_resolver: Arc<dyn SecretResolver> = Arc::new(DispatchSecretResolver::new(
        secret_repo.clone(),
        secret_cipher.clone(),
    ));

    let audit_log: Arc<dyn AuditLog> = Arc::new(PgAuditLog::new(db.clone()));

    let permission_checker = Arc::new(
        CedarPermissionService::new(
            authz_provider.clone(),
            role_repo.clone(),
            grant_repo.clone(),
            audit_log,
        )
        .await
        .map_err(|e| StartupError::Permission(e.to_string()))?,
    );
    let actions = Arc::new(Actions::new(
        Arc::new(PermissionAuthorizer::new(permission_checker.clone())),
        hooks,
    ));

    let auth_uc = Arc::new(AuthUseCases::new(
        user_repo.clone(),
        session_repo.clone(),
        hash_service.clone(),
    ));
    let user_uc = Arc::new(UserUseCases::new(
        user_repo.clone(),
        grant_repo.clone(),
        hash_service.clone(),
        account_repo.clone(),
    ));
    let public_url = config.server.public_url.as_deref();
    if public_url.is_none() {
        tracing::warn!(
            "[server].public_url is not set: a reset link is a relative path. Set it to the \
             origin that people open, for example https://scylla.example.com."
        );
    }
    let reset_uc = Arc::new(PasswordResetUseCases::new(
        user_repo.clone(),
        account_repo.clone(),
        hash_service.clone(),
        reset_sender.clone(),
        ResetLinks::new(public_url),
    ));
    // The registry and the live tail come first: the dispatcher sends through the one and
    // closes the other, and every use case that starts or ends a job goes through the dispatcher.
    let (wakes, woken) = mpsc::unbounded_channel();
    let agent_registry = Arc::new(InMemoryAgentRegistry::new(wakes));
    let job_log_stream = Arc::new(InMemoryJobLogStream::new());
    let dispatch_uc = Arc::new(DispatchUseCases::new(
        agent_registry.clone(),
        permission_checker.clone(),
        job_repo.clone(),
        secret_resolver.clone(),
        job_log_stream.clone(),
    ));
    let org_uc = Arc::new(OrganizationUseCases::new(
        org_repo.clone(),
        user_repo.clone(),
        app_repo.clone(),
        dispatch_uc.clone(),
    ));
    let project_uc = Arc::new(ProjectUseCases::new(
        project_repo.clone(),
        user_repo.clone(),
        permission_checker.clone(),
        dispatch_uc.clone(),
    ));
    let secret_uc = Arc::new(SecretUseCases::new(
        secret_repo.clone(),
        secret_cipher.clone(),
    ));
    let pipeline_uc = Arc::new(PipelineUseCases::new(
        pipeline_repo.clone(),
        project_repo.clone(),
        job_repo.clone(),
        secret_resolver.clone(),
        dispatch_uc.clone(),
    ));
    let job_uc = Arc::new(JobUseCases::new(
        job_repo.clone(),
        job_log_stream.clone(),
        dispatch_uc.clone(),
    ));
    let app_uc = Arc::new(AppUseCases::new(
        app_repo.clone(),
        app_credential_repo.clone(),
        hash_service.clone(),
        agent_registry.clone(),
    ));
    let app_token_uc = Arc::new(AppTokenUseCases::new(
        app_repo.clone(),
        app_token_repo.clone(),
        app_credential_repo.clone(),
        hash_service.clone(),
    ));
    let agent_uc = Arc::new(AgentUseCases::new(
        app_repo.clone(),
        agent_repo.clone(),
        job_repo.clone(),
        hash_service.clone(),
        agent_registry.clone(),
    ));
    let grant_uc = Arc::new(GrantUseCases::new(
        grant_repo.clone(),
        role_repo.clone(),
        agent_registry.clone(),
        authz_provider.clone(),
    ));
    let role_uc = Arc::new(RoleUseCases::new(role_repo.clone(), grant_repo.clone()));

    let job_log_uc = Arc::new(JobLogUseCases::new(
        job_log_repo.clone(),
        job_log_stream.clone(),
        job_repo.clone(),
    ));
    let trigger_repo = Arc::new(PgTriggerRepository::new(db.clone()));
    let trigger_delivery_repo = Arc::new(PgTriggerDeliveryRepository::new(db.clone()));
    let cron_schedule: Arc<dyn CronSchedule> = Arc::new(CronScheduleService::new());
    let trigger_uc = Arc::new(TriggerUseCases::new(
        trigger_repo.clone(),
        pipeline_repo.clone(),
        project_repo.clone(),
        app_repo.clone(),
        secret_cipher.clone(),
        cron_schedule.clone(),
    ));
    let firing: Arc<dyn TriggerFiring> = Arc::new(TriggerFirer::new(
        actions.clone(),
        trigger_uc.clone(),
        pipeline_uc.clone(),
    ));
    let trigger_fire_uc = Arc::new(TriggerFireUseCases::new(firing.clone()));
    let webhook_ingress_uc = Arc::new(WebhookIngressUseCases::new(
        trigger_repo.clone(),
        trigger_delivery_repo.clone(),
        secret_cipher.clone(),
        firing.clone(),
    ));

    // The first tick fires immediately so a pre-restart backlog is picked up; 15s keeps latency under cron's minute.
    if ce.contains(&Worker::CronTriggers) {
        let scheduler = TriggerCronScheduler::new(actions.clone(), trigger_uc.clone(), firing);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(15));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                scheduler.tick().await;
            }
        });
    }

    let dispatch_shared = dispatch_uc.clone();

    // A wake names the agents a pass is for; the tick passes over every agent. The wakes that
    // arrived during a pass make one pass.
    let wakes = if ce.contains(&Worker::DispatchJobs) {
        let scheduler = PendingJobScheduler::new(actions.clone(), dispatch_uc.clone());
        let mut woken = woken;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(30));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                let first = tokio::select! {
                    Some(wake) = woken.recv() => wake,
                    _ = tick.tick() => None,
                };
                scheduler
                    .drain(PendingJobScheduler::targets(first, &mut woken))
                    .await;
            }
        });
        None
    } else {
        Some(woken)
    };

    if ce.contains(&Worker::ReapJobs) {
        let reaper = JobReaper::new(actions.clone(), job_uc.clone());
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(30));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                reaper.reap().await;
            }
        });
    }

    if ce.contains(&Worker::SweepSessions) {
        let sweeper = SessionSweeper::new(actions.clone(), auth_uc.clone());
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(3600));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                sweeper.sweep().await;
            }
        });
    }

    let shared = scylla_strata::Shared {
        db: db.clone(),
        permissions: permission_checker.clone(),
        visibility: permission_checker.clone(),
        sessions: session_repo.clone(),
        app_tokens: app_token_repo.clone(),
        projects: project_repo,
        users: user_repo,
        jobs: job_repo,
        dispatch: dispatch_shared,
        secrets: secret_repo.clone(),
        cipher: secret_cipher.clone(),
        pipelines: pipeline_repo.clone(),
        secret_resolver: secret_resolver.clone(),
        apps: app_repo.clone(),
        app_credentials: app_credential_repo.clone(),
        hash: hash_service.clone(),
        roles: role_repo.clone(),
        grants: grant_repo.clone(),
        authz_entities: authz_provider.clone(),
        organizations: org_repo.clone(),
        accounts: account_repo.clone(),
        reset_sender,
        reset_links: ResetLinks::new(public_url),
        trigger_repo: trigger_repo.clone(),
        trigger_deliveries: trigger_delivery_repo.clone(),
        cron_schedule: cron_schedule.clone(),
        webhook_base_url: config
            .webhook
            .as_ref()
            .and_then(|w| w.public_base_url.clone()),
        job_logs: job_log_repo.clone(),
        job_log_stream: job_log_stream.clone(),
        agents: agent_repo.clone(),
        agent_streams: agent_registry.clone(),
        trust_forwarded_headers: config.server.trust_forwarded_headers,
    };

    Ok(Services {
        shared,
        auth_uc,
        user_uc,
        reset_uc,
        org_uc,
        actions,
        project_uc,
        pipeline_uc,
        trigger_uc,
        trigger_fire_uc,
        webhook_ingress_uc,
        secret_uc,
        job_uc,
        job_log_uc,
        app_uc,
        app_token_uc,
        agent_uc,
        agent_registry,
        grant_uc,
        role_uc,
        permission_checker,
        session_repo,
        app_token_repo,
        wakes,
    })
}

pub(crate) fn build_cors_layer(cors: &scylla_core::config::CorsConfig) -> CorsLayer {
    let mut layer = CorsLayer::new();

    if cors.allow_origins.iter().any(|o| o == "*") {
        tracing::warn!(
            "CORS allow_origins contains '*': any origin is accepted. Do NOT use this in production — set explicit origins in config."
        );
        layer = layer.allow_origin(tower_http::cors::Any);
    } else {
        let origins: Vec<HeaderValue> = cors
            .allow_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        layer = layer.allow_origin(origins);
    }

    let methods: Vec<Method> = cors
        .allow_methods
        .iter()
        .filter_map(|m| m.parse().ok())
        .collect();
    layer = layer.allow_methods(methods);

    let headers: Vec<HeaderName> = cors
        .allow_headers
        .iter()
        .filter_map(|h| h.parse().ok())
        .collect();
    layer = layer.allow_headers(headers);

    layer = layer.max_age(Duration::from_secs(cors.max_age_seconds));

    let expose_headers: Vec<HeaderName> = cors
        .expose_headers
        .iter()
        .filter_map(|h| h.parse().ok())
        .collect();
    layer = layer.expose_headers(expose_headers);

    layer
}

// Client-caused statuses log at DEBUG; the server's own errors are already logged by the error mapper.
pub(crate) fn grpc_classifier() -> GrpcErrorsAsFailures {
    let client = [
        GrpcCode::Cancelled,
        GrpcCode::InvalidArgument,
        GrpcCode::NotFound,
        GrpcCode::AlreadyExists,
        GrpcCode::PermissionDenied,
        GrpcCode::ResourceExhausted,
        GrpcCode::FailedPrecondition,
        GrpcCode::Aborted,
        GrpcCode::OutOfRange,
        GrpcCode::Unauthenticated,
    ];
    client.into_iter().fold(
        GrpcErrorsAsFailures::new(),
        GrpcErrorsAsFailures::with_success,
    )
}

pub(crate) async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => tracing::info!("Received Ctrl+C, shutting down"),
        () = terminate => tracing::info!("Received SIGTERM, shutting down"),
    }
}

/// Adds the CE handler of each service that `strata` does not serve, then mounts the strata
/// services. One table (`Service::slice`) decides both, so no service is added twice.
pub(crate) fn add_services(
    grpc: &mut tonic::service::RoutesBuilder,
    config: &ControlPlaneConfig,
    services: &Services,
    strata: Option<(&dyn Built, &[Slice])>,
    auth_interceptor: &scylla_strata::AuthInterceptorLayer,
    in_flight: &strata::InFlight,
    workers: scylla_strata::Workers,
) {
    use scylla_core::grpc::{
        AgentAdminHandler, AgentHandler, AppAuthHandler, AppHandler, AuthHandler, GrantHandler,
        JobHandler, OrganizationHandler, PipelineHandler, ProjectHandler, RoleHandler,
        SecretHandler, TriggerHandler, UserHandler,
    };
    use scylla_proto::{
        agent::v1::agent_admin_service_server::AgentAdminServiceServer,
        agent::v1::agent_service_server::AgentServiceServer,
        app::v1::app_auth_service_server::AppAuthServiceServer,
        app::v1::app_service_server::AppServiceServer,
        auth::v1::auth_service_server::AuthServiceServer,
        authz::v1::grant_service_server::GrantServiceServer,
        authz::v1::role_service_server::RoleServiceServer,
        job::v1::job_service_server::JobServiceServer,
        organization::v1::organization_service_server::OrganizationServiceServer,
        pipeline::v1::pipeline_service_server::PipelineServiceServer,
        project::v1::project_service_server::ProjectServiceServer,
        secret::v1::secret_service_server::SecretServiceServer,
        trigger::v1::trigger_service_server::TriggerServiceServer,
        user::v1::user_service_server::UserServiceServer,
    };
    use tower::ServiceBuilder;

    let auth_handler = AuthHandler::new(
        services.actions.clone(),
        services.auth_uc.clone(),
        services.reset_uc.clone(),
    )
    .trust_forwarded_headers(config.server.trust_forwarded_headers);
    let user_handler = UserHandler::new(
        services.actions.clone(),
        services.user_uc.clone(),
        services.reset_uc.clone(),
    );
    let org_handler = OrganizationHandler::new(services.actions.clone(), services.org_uc.clone());
    let project_handler =
        ProjectHandler::new(services.actions.clone(), services.project_uc.clone());
    let pipeline_handler =
        PipelineHandler::new(services.actions.clone(), services.pipeline_uc.clone());
    let trigger_handler = TriggerHandler::new(
        services.actions.clone(),
        services.trigger_uc.clone(),
        services.trigger_fire_uc.clone(),
        config
            .webhook
            .as_ref()
            .and_then(|w| w.public_base_url.clone()),
    );
    let job_handler = JobHandler::new(
        services.actions.clone(),
        services.job_uc.clone(),
        services.job_log_uc.clone(),
    );
    let app_handler = AppHandler::new(services.actions.clone(), services.app_uc.clone());
    let secret_handler = SecretHandler::new(services.actions.clone(), services.secret_uc.clone());
    let app_auth_handler =
        AppAuthHandler::new(services.actions.clone(), services.app_token_uc.clone());
    let agent_handler = AgentHandler::new(
        services.actions.clone(),
        services.job_uc.clone(),
        services.job_log_uc.clone(),
        services.agent_uc.clone(),
        services.agent_registry.clone(),
    );
    let agent_admin_handler = AgentAdminHandler::new(
        services.actions.clone(),
        services.agent_uc.clone(),
        services.app_uc.clone(),
    );
    let grant_handler = GrantHandler::new(services.actions.clone(), services.grant_uc.clone());
    let role_handler = RoleHandler::new(services.actions.clone(), services.role_uc.clone());

    let auth_service = AuthServiceServer::new(auth_handler);

    let app_auth_service = AppAuthServiceServer::new(app_auth_handler);

    let user_service = ServiceBuilder::new()
        .layer(auth_interceptor.clone())
        .service(UserServiceServer::new(user_handler));

    let org_service = ServiceBuilder::new()
        .layer(auth_interceptor.clone())
        .service(OrganizationServiceServer::new(org_handler));

    let project_service = ServiceBuilder::new()
        .layer(auth_interceptor.clone())
        .service(ProjectServiceServer::new(project_handler));

    let pipeline_service = ServiceBuilder::new()
        .layer(auth_interceptor.clone())
        .service(PipelineServiceServer::new(pipeline_handler));

    let trigger_service = ServiceBuilder::new()
        .layer(auth_interceptor.clone())
        .service(TriggerServiceServer::new(trigger_handler));

    let job_service = ServiceBuilder::new()
        .layer(auth_interceptor.clone())
        .service(JobServiceServer::new(job_handler));

    let app_service = ServiceBuilder::new()
        .layer(auth_interceptor.clone())
        .service(AppServiceServer::new(app_handler));

    let secret_service = ServiceBuilder::new()
        .layer(auth_interceptor.clone())
        .service(SecretServiceServer::new(secret_handler));

    let agent_service = ServiceBuilder::new()
        .layer(auth_interceptor.clone())
        .service(
            AgentServiceServer::new(agent_handler)
                .max_decoding_message_size(scylla_proto::agent::MAX_MESSAGE_BYTES)
                .max_encoding_message_size(scylla_proto::agent::MAX_MESSAGE_BYTES),
        );

    let agent_admin_service = ServiceBuilder::new()
        .layer(auth_interceptor.clone())
        .service(AgentAdminServiceServer::new(agent_admin_handler));

    let grant_service = ServiceBuilder::new()
        .layer(auth_interceptor.clone())
        .service(GrantServiceServer::new(grant_handler));

    let role_service = ServiceBuilder::new()
        .layer(auth_interceptor.clone())
        .service(RoleServiceServer::new(role_handler));

    let strata_slices = strata.map_or(&[][..], |(_, slices)| slices);
    let ce = |service: Service| !service.on_strata(strata_slices);
    if ce(Service::Auth) {
        grpc.add_service(auth_service);
    }
    if ce(Service::Triggers) {
        grpc.add_service(trigger_service);
    }
    if ce(Service::Jobs) {
        grpc.add_service(job_service);
    }
    if ce(Service::Agents) {
        grpc.add_service(agent_service);
    }
    if ce(Service::AgentAdmin) {
        grpc.add_service(agent_admin_service);
    }
    if ce(Service::Projects) {
        grpc.add_service(project_service);
    }
    if ce(Service::Secrets) {
        grpc.add_service(secret_service);
    }
    if ce(Service::Users) {
        grpc.add_service(user_service);
    }
    if ce(Service::Organizations) {
        grpc.add_service(org_service);
    }
    if ce(Service::Grants) {
        grpc.add_service(grant_service);
    }
    if ce(Service::Roles) {
        grpc.add_service(role_service);
    }
    if ce(Service::Pipelines) {
        grpc.add_service(pipeline_service);
    }
    if ce(Service::Apps) {
        grpc.add_service(app_service);
    }
    if ce(Service::AppAuth) {
        grpc.add_service(app_auth_service);
    }
    if let Some((built, slices)) = strata {
        built.mount(slices, grpc, auth_interceptor, in_flight, workers);
    }
}

/// `GrpcWebLayer` answers 400 to any non-gRPC-Web HTTP/1.1 request, so it is scoped to the gRPC routes;
/// `Router::layer` wraps the fallback too, so the UI fallback is attached last.
pub(crate) async fn run_server<F>(
    config: &ControlPlaneConfig,
    services: &Services,
    surface: Surface,
    strata: Option<(&dyn Built, &[Slice])>,
    in_flight: &strata::InFlight,
    workers: scylla_strata::Workers,
    shutdown: F,
) -> Result<(), StartupError>
where
    F: Future<Output = ()> + Send + 'static,
{
    use tonic::service::Routes;
    use tonic::transport::Server;
    use tonic_web::GrpcWebLayer;

    let auth_interceptor = async_interceptor(AuthInterceptor::new(
        services.session_repo.clone(),
        services.app_token_repo.clone(),
    ));
    // v1 and v1alpha reflection both: grpcurl and some bridges default to v1alpha.
    let reflection = || {
        let mut builder = tonic_reflection::server::Builder::configure()
            .register_encoded_file_descriptor_set(scylla_proto::FILE_DESCRIPTOR_SET);
        for set in &surface.descriptors {
            builder = builder.register_encoded_file_descriptor_set(set);
        }
        builder
    };
    let reflection_v1 = reflection()
        .build_v1()
        .map_err(|e| StartupError::Reflection(e.to_string()))?;
    let reflection_v1alpha = reflection()
        .build_v1alpha()
        .map_err(|e| StartupError::Reflection(e.to_string()))?;

    let mut grpc = Routes::builder();
    grpc.add_service(reflection_v1)
        .add_service(reflection_v1alpha);
    add_services(
        &mut grpc,
        config,
        services,
        strata,
        &auth_interceptor,
        in_flight,
        workers,
    );
    let strata_slices = strata.map_or(&[][..], |(_, slices)| slices);

    for add in surface.grpc {
        add(&mut grpc, &auth_interceptor);
    }

    // Scoped to the gRPC routes: `GrpcWebLayer` 400s everything else, and the gRPC classifier reads `grpc-status`.
    let grpc = grpc
        .routes()
        .into_axum_router()
        .layer(GrpcWebLayer::new())
        .layer(TraceLayer::new(SharedClassifier::new(grpc_classifier())));

    // The webhook router sets no fallback, so merging into the tonic router (which has one) is safe.
    let http = surface
        .http
        .into_iter()
        .fold(
            webhook_router(services, strata.map(|(built, _)| built), strata_slices),
            axum::Router::merge,
        )
        .layer(TraceLayer::new_for_http());

    let app = grpc.merge(http).merge(scylla_core::rest::health::router());

    // Must stay last.
    let app = scylla_core::rest::ui::attach(app, &config.ui);

    // CORS is no longer needed by the UI (same origin) but third-party clients and Vite on :5173 rely on it.
    // The keepalive ends a dead agent connection in about 30 seconds.
    let mut server = Server::builder()
        .accept_http1(true)
        .http2_keepalive_interval(Some(Duration::from_secs(20)))
        .http2_keepalive_timeout(Some(Duration::from_secs(10)))
        .layer(build_cors_layer(&config.cors));
    let router = server.add_routes(Routes::from(app));

    match config.server.tls.as_ref() {
        None => {
            tracing::info!("server listening on http://{}", config.server.address);
            router
                .serve_with_shutdown(config.server.address, shutdown)
                .await?;
        }
        Some(tls) => {
            let incoming =
                scylla_core::tls::incoming(config.server.address, scylla_core::tls::acceptor(tls)?)
                    .await?;
            tracing::info!("server listening on https://{}", config.server.address);
            router
                .serve_with_incoming_shutdown(incoming, shutdown)
                .await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ce_workers, grpc_classifier};
    use scylla_strata::{Slice, Worker};
    use tower_http::classify::{ClassifiedResponse, ClassifyResponse};

    #[test]
    fn a_switched_slice_takes_its_workers_off_ce() {
        assert_eq!(ce_workers(&[]), Worker::ALL);
        assert!(ce_workers(Slice::ALL).is_empty());
        assert_eq!(
            ce_workers(&[Slice::Jobs, Slice::Projects]),
            [Worker::CronTriggers, Worker::SweepSessions]
        );
        assert_eq!(
            ce_workers(&[Slice::Auth, Slice::Triggers]),
            [Worker::DispatchJobs, Worker::ReapJobs]
        );
    }

    fn fails(code: u16) -> bool {
        let response = http::Response::builder()
            .header("grpc-status", code)
            .body(())
            .unwrap();
        matches!(
            grpc_classifier().classify_response(&response),
            ClassifiedResponse::Ready(Err(_))
        )
    }

    #[test]
    fn client_statuses_are_not_failures() {
        for code in [0, 1, 3, 5, 6, 7, 8, 9, 10, 11, 16] {
            assert!(!fails(code), "grpc-status {code}");
        }
    }

    #[test]
    fn server_statuses_are_failures() {
        for code in [2, 4, 12, 13, 14, 15] {
            assert!(fails(code), "grpc-status {code}");
        }
    }
}

#[cfg(test)]
mod switched_tests {
    use super::{Services, bootstrap, init_services, webhook_router};
    use async_trait::async_trait;
    use axum::body::Body;
    use http::{Request, StatusCode};
    use scylla_auth::authz::{Principal, SYSTEM_ADMIN_ROLE, Scope};
    use scylla_core::config::{BootstrapConfig, ControlPlaneConfig};
    use scylla_core::domain::errors::DomainError;
    use scylla_core::domain::user::{Email, Username};
    use scylla_core::infrastructure::LogPasswordResetSender;
    use scylla_extension::{Action, Hooks, Observer, StageKind};
    use scylla_strata::{Built, EditionHost, Host, ScyllaCeOnDb, Slice};
    use sqlx::PgPool;
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;

    /// Keeps the caller of each bootstrap or webhook action that runs on the CE Actions. The CE
    /// workers of the other slices are not kept.
    #[derive(Default)]
    struct Seen(Mutex<Vec<String>>);

    #[async_trait]
    impl Observer for Seen {
        async fn observe(&self, _: StageKind, action: &dyn Action, _: Result<(), &DomainError>) {
            let caller = action.caller().to_string();
            if caller == "service:bootstrap" || caller == "anonymous" {
                self.0.lock().unwrap().push(caller);
            }
        }
    }

    fn config() -> ControlPlaneConfig {
        ControlPlaneConfig {
            bootstrap: Some(BootstrapConfig {
                username: "root".to_string(),
                password: "SecurePass123!".to_string(),
                email: "root@example.com".to_string(),
            }),
            ..ControlPlaneConfig::default()
        }
    }

    async fn start(
        db: PgPool,
        switched: &[Slice],
    ) -> (Services, Option<Box<dyn Built>>, Arc<Seen>) {
        let seen = Arc::new(Seen::default());
        let mut hooks = Hooks::new();
        hooks.observe(StageKind::Authorize, seen.clone());
        let services = init_services(
            &config(),
            db,
            Arc::new(hooks),
            Arc::new(LogPasswordResetSender),
            switched,
        )
        .await
        .unwrap();
        let built = if switched.is_empty() {
            None
        } else {
            let host = EditionHost::<ScyllaCeOnDb>::new(Slice::ALL);
            Some(host.build(&services.shared).await.unwrap())
        };
        (services, built, seen)
    }

    async fn deliver(
        services: &Services,
        built: Option<&dyn Built>,
        switched: &[Slice],
    ) -> StatusCode {
        let request = Request::post("/webhooks/missing")
            .header("x-scylla-signature-256", "sha256=00")
            .body(Body::from("{}"))
            .unwrap();
        webhook_router(services, built, switched)
            .oneshot(request)
            .await
            .unwrap()
            .status()
    }

    async fn assert_one_admin(services: &Services) {
        let admin = services
            .shared
            .users
            .find_by_username(&Username::new("root").unwrap())
            .await
            .unwrap();
        assert_eq!(admin.email().map(Email::as_str), Some("root@example.com"));
        let grants: Vec<_> = services
            .shared
            .grants
            .list_all()
            .await
            .unwrap()
            .into_iter()
            .filter(|g| g.principal == Principal::User(admin.id().clone()))
            .collect();
        assert_eq!(grants.len(), 1, "{grants:?}");
        assert_eq!(grants[0].scope, Scope::System);
        assert_eq!(grants[0].role.as_str(), SYSTEM_ADMIN_ROLE);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn switched_slices_run_the_bootstrap_and_the_webhook_off_the_actions(db: PgPool) {
        let switched = [Slice::Users, Slice::Grants, Slice::Triggers];
        let (services, built, seen) = start(db, &switched).await;

        bootstrap(&config(), &services, built.as_deref(), &switched)
            .await
            .unwrap();
        bootstrap(&config(), &services, built.as_deref(), &switched)
            .await
            .unwrap();
        let status = deliver(&services, built.as_deref(), &switched).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_one_admin(&services).await;
        assert!(seen.0.lock().unwrap().is_empty(), "{:?}", seen.0);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn unswitched_slices_run_the_bootstrap_and_the_webhook_on_the_actions(db: PgPool) {
        let (services, built, seen) = start(db, &[]).await;

        bootstrap(&config(), &services, built.as_deref(), &[])
            .await
            .unwrap();
        bootstrap(&config(), &services, built.as_deref(), &[])
            .await
            .unwrap();
        let status = deliver(&services, built.as_deref(), &[]).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_one_admin(&services).await;
        let seen = seen.0.lock().unwrap();
        assert!(seen.iter().any(|c| c.contains("bootstrap")), "{seen:?}");
        assert!(seen.iter().any(|c| c == "anonymous"), "{seen:?}");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_bootstrap_needs_users_and_grants_switched_to_leave_the_actions(db: PgPool) {
        let switched = [Slice::Users, Slice::Triggers];
        let (services, built, seen) = start(db, &switched).await;

        bootstrap(&config(), &services, built.as_deref(), &switched)
            .await
            .unwrap();
        let status = deliver(&services, built.as_deref(), &switched).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_one_admin(&services).await;
        let seen = seen.0.lock().unwrap();
        assert!(!seen.is_empty());
        assert!(seen.iter().all(|c| c.contains("bootstrap")), "{seen:?}");
    }
}

#[cfg(test)]
mod routes_tests {
    use super::{Services, add_services, init_services};
    use axum::body::Body;
    use http::{Request, StatusCode};
    use scylla_core::config::ControlPlaneConfig;
    use scylla_core::grpc::auth_interceptor::AuthInterceptor;
    use scylla_core::infrastructure::LogPasswordResetSender;
    use scylla_extension::Hooks;
    use scylla_strata::{Agents, Apps, EditionHost, Host, ScyllaCeOnDb, Service, Slice, Workers};
    use sqlx::PgPool;
    use std::sync::Arc;
    use tonic_async_interceptor::async_interceptor;
    use tower::ServiceExt;

    fn method(service: Service) -> &'static str {
        match service {
            Service::Projects => "GetProject",
            Service::Secrets => "ListSecrets",
            Service::Pipelines => "GetPipeline",
            Service::Apps => "GetApp",
            Service::AppAuth => "IssueToken",
            Service::Roles => "GetRole",
            Service::Grants => "RevokeGrant",
            Service::Organizations => "GetOrganization",
            Service::Users => "GetUser",
            Service::Auth => "Login",
            Service::Triggers => "GetTrigger",
            Service::Jobs => "GetJob",
            Service::Agents => "Open",
            Service::AgentAdmin => "GetAgent",
        }
    }

    async fn services(db: PgPool, switched: &[Slice]) -> Services {
        init_services(
            &ControlPlaneConfig::default(),
            db,
            Arc::new(Hooks::default()),
            Arc::new(LogPasswordResetSender),
            switched,
        )
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn each_new_slice_alone_leaves_every_service_answering(db: PgPool) {
        for slice in [Slice::Auth, Slice::Triggers, Slice::Jobs, Slice::Agents] {
            let switched = [slice];
            let mut services = services(db.clone(), &switched).await;
            let built = EditionHost::<ScyllaCeOnDb>::new(Slice::ALL)
                .build(&services.shared)
                .await
                .unwrap();
            let auth = async_interceptor(AuthInterceptor::new(
                services.session_repo.clone(),
                services.app_token_repo.clone(),
            ));
            let workers = Workers {
                wakes: services.wakes.take(),
                ..Workers::default()
            };
            let stop = workers.stop.clone();
            let mut grpc = tonic::service::Routes::builder();
            add_services(
                &mut grpc,
                &ControlPlaneConfig::default(),
                &services,
                Some((&*built, &switched)),
                &auth,
                &strata::InFlight::default(),
                workers,
            );
            let router = grpc.routes().into_axum_router();
            for service in Service::ALL {
                let request = Request::post(format!("/{}/{}", service.name(), method(*service)))
                    .header("content-type", "application/grpc")
                    .header("te", "trailers")
                    .body(Body::from(vec![0u8; 5]))
                    .unwrap();
                let response = router.clone().oneshot(request).await.unwrap();
                assert_eq!(response.status(), StatusCode::OK, "{slice} {service:?}");
                let status = response
                    .headers()
                    .get("grpc-status")
                    .map(|v| v.to_str().unwrap().to_owned());
                assert!(status.is_some(), "{slice} {service:?} gave no status");
                assert_ne!(status.as_deref(), Some("12"), "{slice} {service:?}");
            }
            stop.cancel();
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn the_apps_slot_and_the_agent_stream_share_the_ce_registry(db: PgPool) {
        let services = services(db, Slice::ALL).await;
        let app = EditionHost::<ScyllaCeOnDb>::new(Slice::ALL)
            .edition(&services.shared, true)
            .await
            .unwrap();

        let slot = Arc::as_ptr(Apps::app_registry(&*app));
        let streams = Arc::as_ptr(Agents::agent_links(&*app).registry());
        let ce = Arc::as_ptr(&services.agent_registry);

        assert!(std::ptr::addr_eq(slot, streams));
        assert!(std::ptr::addr_eq(streams, ce));
    }
}
