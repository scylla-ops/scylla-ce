use crate::surface::Surface;
use http::{HeaderName, HeaderValue, Method};
use scylla_auth::audit::AuditLog;
use scylla_auth::cedar::CedarPermissionService;
use scylla_core::application::user::reset::ResetLinks;
use scylla_core::application::{
    CronSchedule, DispatchSecretResolver, DispatchUseCases, PasswordResetSender, SecretCipher,
    SecretResolver,
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
use scylla_strata::{Built, EditionHost, Host, ScyllaCeOnDb, Slice};
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
    pub permission_checker: Arc<CedarPermissionService<PgAuthzEntityProvider>>,
    pub session_repo: Arc<PgSessionRepository>,
    pub app_token_repo: Arc<PgAppTokenRepository>,
    pub wakes: Option<mpsc::UnboundedReceiver<Option<AppId>>>,
}

/// The CE adapters that a strata edition takes (`EditionHost::edition`), on `db`, with no worker
/// started. When `config` has a `[bootstrap]` section, the CE edition runs it.
pub async fn strata_shared(
    config: &ControlPlaneConfig,
    db: PgPool,
    reset_sender: Arc<dyn PasswordResetSender>,
) -> Result<scylla_strata::Shared, StartupError> {
    let services = init_services(config, db, reset_sender).await?;
    if config.bootstrap.is_some() {
        let built = EditionHost::<ScyllaCeOnDb>::new(Slice::ALL)
            .build(&services.shared)
            .await
            .map_err(|e| StartupError::Strata(format!("{e:#}")))?;
        bootstrap(config, &*built).await?;
    }
    Ok(services.shared)
}

/// Runs the admin bootstrap of `config` on the Users and Grants modules of `built`.
pub(crate) async fn bootstrap(
    config: &ControlPlaneConfig,
    built: &dyn Built,
) -> Result<(), StartupError> {
    if let Some(cfg) = &config.bootstrap {
        built.bootstrap(cfg).await?;
    }
    Ok(())
}

/// The webhook route, on the Triggers module of `built`.
pub(crate) fn webhook_router(built: &dyn Built) -> axum::Router {
    scylla_core::rest::webhook::router(built.webhooks())
}

pub(crate) async fn init_services(
    config: &ControlPlaneConfig,
    db: PgPool,
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

    let public_url = config.server.public_url.as_deref();
    if public_url.is_none() {
        tracing::warn!(
            "[server].public_url is not set: a reset link is a relative path. Set it to the \
             origin that people open, for example https://scylla.example.com."
        );
    }
    // The registry and the live tail come first: the dispatcher sends through the one and
    // closes the other.
    let (wakes, woken) = mpsc::unbounded_channel();
    let agent_registry = Arc::new(InMemoryAgentRegistry::new(wakes));
    let job_log_stream = Arc::new(InMemoryJobLogStream::new());
    let dispatch = Arc::new(DispatchUseCases::new(
        agent_registry.clone(),
        job_repo.clone(),
        job_log_stream.clone(),
    ));
    let trigger_repo = Arc::new(PgTriggerRepository::new(db.clone()));
    let trigger_delivery_repo = Arc::new(PgTriggerDeliveryRepository::new(db.clone()));
    let cron_schedule: Arc<dyn CronSchedule> = Arc::new(CronScheduleService::new());

    let shared = scylla_strata::Shared {
        db: db.clone(),
        permissions: permission_checker.clone(),
        visibility: permission_checker.clone(),
        sessions: session_repo.clone(),
        app_tokens: app_token_repo.clone(),
        projects: project_repo,
        users: user_repo,
        jobs: job_repo,
        dispatch,
        secrets: secret_repo,
        cipher: secret_cipher,
        pipelines: pipeline_repo,
        secret_resolver,
        apps: app_repo,
        app_credentials: app_credential_repo,
        hash: hash_service,
        roles: role_repo,
        grants: grant_repo,
        authz_entities: authz_provider,
        organizations: org_repo,
        accounts: account_repo,
        reset_sender,
        reset_links: ResetLinks::new(public_url),
        trigger_repo,
        trigger_deliveries: trigger_delivery_repo,
        cron_schedule,
        webhook_base_url: config
            .webhook
            .as_ref()
            .and_then(|w| w.public_base_url.clone()),
        job_logs: job_log_repo,
        job_log_stream,
        agents: agent_repo,
        agent_streams: agent_registry,
        trust_forwarded_headers: config.server.trust_forwarded_headers,
    };

    Ok(Services {
        shared,
        permission_checker,
        session_repo,
        app_token_repo,
        wakes: Some(woken),
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
    built: &dyn Built,
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
    built.mount(Slice::ALL, &mut grpc, &auth_interceptor, in_flight, workers);

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
        .fold(webhook_router(built), axum::Router::merge)
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

#[cfg(test)]
mod strata_tests {
    use super::{Services, bootstrap, init_services, webhook_router};
    use axum::body::Body;
    use http::{Request, StatusCode};
    use scylla_auth::authz::{Principal, SYSTEM_ADMIN_ROLE, Scope};
    use scylla_core::config::{BootstrapConfig, ControlPlaneConfig};
    use scylla_core::domain::user::{Email, Username};
    use scylla_core::grpc::auth_interceptor::AuthInterceptor;
    use scylla_core::infrastructure::LogPasswordResetSender;
    use scylla_strata::{
        Agents, Apps, Built, EditionHost, Host, ScyllaCeOnDb, Service, Slice, Workers,
    };
    use sqlx::PgPool;
    use std::sync::Arc;
    use tonic_async_interceptor::async_interceptor;
    use tower::ServiceExt;

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

    async fn start(db: PgPool) -> (Services, Box<dyn Built>) {
        let services = init_services(&config(), db, Arc::new(LogPasswordResetSender))
            .await
            .unwrap();
        let built = EditionHost::<ScyllaCeOnDb>::new(Slice::ALL)
            .build(&services.shared)
            .await
            .unwrap();
        (services, built)
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
    async fn the_bootstrap_and_the_webhook_run_on_the_edition(db: PgPool) {
        let (services, built) = start(db).await;

        bootstrap(&config(), &*built).await.unwrap();
        bootstrap(&config(), &*built).await.unwrap();
        let request = Request::post("/webhooks/missing")
            .header("x-scylla-signature-256", "sha256=00")
            .body(Body::from("{}"))
            .unwrap();
        let status = webhook_router(&*built)
            .oneshot(request)
            .await
            .unwrap()
            .status();

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_one_admin(&services).await;
    }

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

    #[sqlx::test(migrations = "../../migrations")]
    async fn the_edition_answers_every_service(db: PgPool) {
        let (mut services, built) = start(db).await;
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
        built.mount(
            Slice::ALL,
            &mut grpc,
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
            assert_eq!(response.status(), StatusCode::OK, "{service:?}");
            let status = response
                .headers()
                .get("grpc-status")
                .map(|v| v.to_str().unwrap().to_owned());
            assert!(status.is_some(), "{service:?} gave no status");
            assert_ne!(status.as_deref(), Some("12"), "{service:?}");
        }
        stop.cancel();
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn the_apps_slot_and_the_agent_stream_share_the_ce_registry(db: PgPool) {
        let (services, _) = start(db).await;
        let app = EditionHost::<ScyllaCeOnDb>::new(Slice::ALL)
            .edition(&services.shared, true)
            .await
            .unwrap();

        let slot = Arc::as_ptr(Apps::app_registry(&*app));
        let streams = Arc::as_ptr(Agents::agent_links(&*app).registry());
        let ce = Arc::as_ptr(&services.shared.agent_streams);

        assert!(std::ptr::addr_eq(slot, streams));
        assert!(std::ptr::addr_eq(streams, ce));
    }
}
