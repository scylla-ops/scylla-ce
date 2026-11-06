//! The gRPC server: gate, operations, gRPC-Web, CORS, TLS, health, the core services, the
//! browser routes of the sign-in, the log of each call, timeout and graceful shutdown.

use std::fs;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use scylla_extension::{Edition, Gate, Mailer, Notifier};
use sqlx::PgPool;
use tokio::net::TcpListener;
use tonic::service::{Routes, RoutesBuilder};
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Identity, Server, ServerTlsConfig};
use tonic_health::ServingStatus;
use tonic_web::GrpcWebLayer;

use super::request_log::RequestLog;
use crate::access::{Access, OperationLayer};
use crate::features::identity::accounts::{self, AccountContext, AccountSettings};
use crate::features::identity::agents::{self, Registry};
use crate::features::identity::providers::{self, Providers};
use crate::features::identity::tokens;
use crate::features::identity::{admins, apps, personal_tokens};
use crate::features::notifications::{self, PgNotifier};
use crate::features::platform::settings;
use crate::features::projects::jobs::{self, housekeeping};
use crate::features::projects::triggers::webhook::{self, Webhooks};
use crate::features::projects::triggers::{self, schedule};
use crate::features::projects::{pipelines, secrets};
use crate::features::tenancy::invitations;
use crate::infra::mail::{LogMailer, SmtpMailer};
use crate::infra::secret::Sealer;
use crate::infra::{db, dev_keys};
use crate::{Config, Error, SignUpMode};

/// The options of `serve`.
#[derive(Clone)]
pub struct ServeOptions {
    /// The time after which the server stops a request.
    pub request_timeout: Duration,
    /// The certificate and key of the server. `None` serves without TLS.
    pub tls: Option<Identity>,
    /// The web origins that can call the API with gRPC-Web.
    pub cors_origins: Vec<String>,
    /// The settings of the accounts and of the sessions.
    pub accounts: AccountSettings,
    /// The mailer of the account mails.
    pub mailer: Arc<dyn Mailer>,
    /// The key of the secrets that the server reads back: the client secrets of the identity
    /// providers and the values of the secrets of the projects.
    pub sealer: Option<Arc<Sealer>>,
    /// The time that the server keeps the logs of the jobs.
    pub log_retention: Duration,
    /// The time that the server keeps a notification after its last fact.
    pub notification_retention: Duration,
    /// The folder of the built web app, if the server serves it.
    pub web_dir: Option<std::path::PathBuf>,
    /// The folder of the downloads of the agent, for the install scripts.
    pub agent_downloads: String,
}

impl Default for ServeOptions {
    fn default() -> Self {
        let local = "http://127.0.0.1:3000".to_owned();
        Self {
            request_timeout: Duration::from_secs(15),
            tls: None,
            cors_origins: Vec::new(),
            accounts: AccountSettings {
                api_url: local.clone(),
                app_url: local,
                sign_up_mode: SignUpMode::InviteOnly,
                session_ttl: Duration::from_secs(30 * 24 * 60 * 60),
                web_origins: Vec::new(),
                trust_forwarded: false,
            },
            mailer: Arc::new(LogMailer),
            sealer: None,
            log_retention: Duration::from_secs(30 * 24 * 60 * 60),
            notification_retention: Duration::from_secs(90 * 24 * 60 * 60),
            web_dir: None,
            agent_downloads: crate::config::AGENT_DOWNLOADS.to_owned(),
        }
    }
}

/// Checks the schema, then serves on the address of the config until SIGTERM or Ctrl+C.
pub async fn run(config: &Config, edition: Edition) -> Result<(), Error> {
    // Refuse a public dev key on an address that other machines can reach.
    dev_keys::guard(config)?;

    // Read the optional parts: TLS, SMTP and the key of the secrets.
    let tls = match (&config.tls_cert, &config.tls_key) {
        (Some(cert), Some(key)) => Some(Identity::from_pem(fs::read(cert)?, fs::read(key)?)),
        _ => None,
    };
    let mailer: Arc<dyn Mailer> =
        match &config.smtp_url {
            Some(url) => Arc::new(SmtpMailer::new(url, &config.mail_from).map_err(|reason| {
                Error::Config {
                    name: "SCYLLA_SMTP_URL",
                    reason,
                }
            })?),
            None => Arc::new(LogMailer),
        };
    let sealer = sealer(config)?;

    // Connect, refuse a database with a missing migration, and make the gate.
    let pool = db::connect(config).await?;
    db::check_migrated(&pool, edition.migrators()).await?;
    let schema = edition.authz_model().clone();
    let access = Arc::new(Access::from_config(config, pool.clone(), schema)?);

    // Bind the address and serve until the stop signal.
    let listener = TcpListener::bind(config.bind_addr).await?;
    tracing::info!(
        edition = edition.name(),
        addr = %listener.local_addr()?,
        tls = tls.is_some(),
        smtp = config.smtp_url.is_some(),
        "listening"
    );
    let options = ServeOptions {
        request_timeout: config.request_timeout,
        tls,
        cors_origins: config.cors_origins.clone(),
        accounts: AccountSettings::from_config(config),
        mailer,
        sealer,
        log_retention: config.log_retention,
        notification_retention: config.notification_retention,
        web_dir: config.web_dir.clone(),
        agent_downloads: config.agent_downloads_url.trim_end_matches('/').to_owned(),
    };
    let stop = super::layers::stop_signal();
    serve(listener, pool.clone(), edition, access, options, stop).await?;

    // Close the pool after the last request.
    pool.close().await;
    tracing::info!("stopped");
    Ok(())
}

/// Reads the key of the secrets, if the settings give one.
pub fn sealer(config: &Config) -> Result<Option<Arc<Sealer>>, Error> {
    let Some(key) = &config.secret_key else {
        return Ok(None);
    };
    let sealer = Sealer::from_hex(key).map_err(|reason| Error::Config {
        name: "SCYLLA_SECRET_KEY",
        reason,
    })?;
    Ok(Some(Arc::new(sealer)))
}

/// Serves on a bound listener until `stop`, then waits for the running requests.
pub async fn serve(
    listener: TcpListener,
    pool: PgPool,
    edition: Edition,
    access: Arc<Access>,
    options: ServeOptions,
    stop: impl Future<Output = ()>,
) -> Result<(), Error> {
    // Make what the account routes share, and the clients of the identity providers.
    let log = RequestLog::new(options.accounts.trust_forwarded);
    let install = agents::install_router(&options.accounts.api_url, &options.agent_downloads);
    let gate: Arc<dyn Gate> = access;
    let ctx = AccountContext {
        schema: edition.authz_model().clone(),
        pool: pool.clone(),
        gate: Arc::clone(&gate),
        mailer: options.mailer,
        settings: Arc::new(options.accounts),
        sealer: options.sealer.clone(),
        grants: edition.grants(),
        hooks: edition.hooks(),
    };
    let providers = Providers::new(ctx.clone());

    // Add the services: health, the core services, then the services of the edition.
    let (health, health_service) = tonic_health::server::health_reporter();
    let mut routes = RoutesBuilder::default();
    routes.add_service(health_service);
    routes.add_service(tokens::server(providers.clone()));
    routes.add_service(accounts::account_server(ctx.clone()));
    routes.add_service(accounts::profile_server(ctx.clone()));
    routes.add_service(admins::server(ctx.clone()));
    routes.add_service(personal_tokens::server(ctx.clone()));
    routes.add_service(settings::server(pool.clone(), Arc::clone(&gate)));
    routes.add_service(invitations::server(ctx));
    routes.add_service(secrets::server(
        pool.clone(),
        Arc::clone(&gate),
        options.sealer.clone(),
    ));

    // Add the services of the Apps and of the jobs: they share the open streams of the agents.
    let registry = Arc::new(Registry::default());
    let (hooks, share) = (edition.hooks(), || Arc::clone(&registry));
    routes.add_service(apps::server(pool.clone(), Arc::clone(&gate), share()));
    let sealer = || options.sealer.clone();
    let agents = agents::server(pool.clone(), Arc::clone(&gate), share(), sealer());
    routes.add_service(agents);
    let pipelines = pipelines::server(pool.clone(), Arc::clone(&hooks), Arc::clone(&gate), share());
    routes.add_service(pipelines);
    let jobs = jobs::server(pool.clone(), Arc::clone(&hooks), Arc::clone(&gate), share());
    routes.add_service(jobs);
    let triggers = triggers::server(
        pool.clone(),
        Arc::clone(&hooks),
        Arc::clone(&gate),
        share(),
        sealer(),
    );
    routes.add_service(triggers);

    // Start the background tasks: the lost agents, the old logs, and the cron triggers.
    let housekeeping = housekeeping::spawn(pool.clone(), share(), options.log_retention);
    let schedules = schedule::spawn(pool.clone(), Arc::clone(&hooks), share());
    let webhooks = Webhooks {
        pool: pool.clone(),
        hooks,
        registry: share(),
        sealer: sealer(),
        rates: Arc::default(),
    };
    let notifier: Arc<dyn Notifier> = Arc::new(PgNotifier { pool: pool.clone() });
    edition.add_services(&mut routes, &pool, &gate, &notifier);
    let notifications = notifications::spawn(
        pool.clone(),
        Arc::clone(&gate),
        edition.notification_kinds(),
        options.notification_retention,
    );

    // Log the calls inside gRPC-Web, then add the browser routes and the JSON gateway.
    let grpc = routes
        .routes()
        .into_axum_router()
        .layer(OperationLayer)
        .layer(log)
        .layer(GrpcWebLayer::new());
    let gateway = super::json::router(grpc.clone(), edition.descriptor_sets());
    let mut router = grpc
        .merge(providers::router(providers).layer(log))
        .merge(webhook::router(webhooks).layer(log))
        .merge(install.layer(log))
        .merge(gateway);

    // Serve the web app for each other GET: the folder of the settings, or the files of the binary.
    if let Some(app) = super::web::app(options.web_dir.as_deref(), edition.web_files()) {
        router = router.fallback_service(app);
    }

    // At the stop signal, report `NOT_SERVING` first.
    let stop = async move {
        stop.await;
        health
            .set_service_status("", ServingStatus::NotServing)
            .await;
        tracing::info!("stopping, waiting for the running requests");
    };

    // Serve gRPC, gRPC-Web and HTTP through the layers, from the outer one to the inner one.
    let mut server = Server::builder().accept_http1(true);
    if let Some(identity) = options.tls {
        server = server.tls_config(ServerTlsConfig::new().identity(identity))?;
    }
    server
        .timeout(options.request_timeout)
        .layer(super::layers::cors_layer(&options.cors_origins))
        .add_routes(Routes::from(router))
        .serve_with_incoming_shutdown(TcpIncoming::from(listener), stop)
        .await?;
    housekeeping.abort();
    schedules.abort();
    notifications.abort();
    Ok(())
}
