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
}

pub(crate) async fn init_services(
    config: &ControlPlaneConfig,
    db: PgPool,
    hooks: Arc<Hooks>,
    reset_sender: Arc<dyn PasswordResetSender>,
) -> Result<Services, StartupError> {
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
        reset_sender,
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
    if let Some(cfg) = &config.bootstrap {
        let bootstrap_uc =
            BootstrapUseCases::new(actions.clone(), user_uc.clone(), grant_uc.clone());
        scylla_core::bootstrap::bootstrap_admin(&bootstrap_uc, cfg).await?;
    }

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
    {
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

    // A wake names the agents a pass is for; the tick passes over every agent. The wakes that
    // arrived during a pass make one pass.
    {
        let scheduler = PendingJobScheduler::new(actions.clone(), dispatch_uc);
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
    }

    {
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

    {
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

    Ok(Services {
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

/// `GrpcWebLayer` answers 400 to any non-gRPC-Web HTTP/1.1 request, so it is scoped to the gRPC routes;
/// `Router::layer` wraps the fallback too, so the UI fallback is attached last.
pub(crate) async fn run_server<F>(
    config: &ControlPlaneConfig,
    services: &Services,
    surface: Surface,
    shutdown: F,
) -> Result<(), StartupError>
where
    F: Future<Output = ()> + Send + 'static,
{
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
    use tonic::service::Routes;
    use tonic::transport::Server;
    use tonic_web::GrpcWebLayer;
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

    let mut grpc = Routes::builder();
    grpc.add_service(reflection_v1)
        .add_service(reflection_v1alpha)
        .add_service(auth_service)
        .add_service(app_auth_service)
        .add_service(user_service)
        .add_service(org_service)
        .add_service(project_service)
        .add_service(pipeline_service)
        .add_service(trigger_service)
        .add_service(secret_service)
        .add_service(job_service)
        .add_service(app_service)
        .add_service(agent_service)
        .add_service(agent_admin_service)
        .add_service(grant_service)
        .add_service(role_service);

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
            scylla_core::rest::webhook::router(
                services.actions.clone(),
                services.webhook_ingress_uc.clone(),
            ),
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
    use super::grpc_classifier;
    use tower_http::classify::{ClassifiedResponse, ClassifyResponse};

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
