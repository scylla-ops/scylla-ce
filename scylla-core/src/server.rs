//! The gRPC server: gate, operations, gRPC-Web, CORS, TLS, health, the core services, the
//! browser routes of the sign-in, tracing, timeout and graceful shutdown.

mod layers;

use std::fs;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use scylla_extension::{Edition, Gate, Mailer};
use sqlx::PgPool;
use tokio::net::TcpListener;
use tonic::service::{Routes, RoutesBuilder};
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Identity, Server, ServerTlsConfig};
use tonic_health::ServingStatus;
use tonic_web::GrpcWebLayer;

use crate::access::{Access, OperationLayer};
use crate::features::accounts::{self, AccountContext, AccountSettings};
use crate::features::providers::{self, Providers};
use crate::features::{invitations, tokens};
use crate::mail::{LogMailer, SmtpMailer};
use crate::secret::Sealer;
use crate::{db, Config, Error, SignUpMode};

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
    /// The key of the client secrets of the identity providers.
    pub sealer: Option<Arc<Sealer>>,
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
            },
            mailer: Arc::new(LogMailer),
            sealer: None,
        }
    }
}

/// Checks the schema, then serves on the address of the config until SIGTERM or Ctrl+C.
pub async fn run(config: &Config, edition: Edition) -> Result<(), Error> {
    // Read the optional parts: TLS, SMTP and the key of the client secrets.
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
    };
    let stop = layers::stop_signal();
    serve(listener, pool.clone(), edition, access, options, stop).await?;

    // Close the pool after the last request.
    pool.close().await;
    tracing::info!("stopped");
    Ok(())
}

/// Reads the key of the client secrets, if the settings give one.
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
    let gate: Arc<dyn Gate> = access;
    let ctx = AccountContext {
        pool: pool.clone(),
        gate: Arc::clone(&gate),
        mailer: options.mailer,
        settings: Arc::new(options.accounts),
        sealer: options.sealer,
        grants: edition.grants(),
    };
    let providers = Providers::new(ctx.clone());

    // Add the services: health, the core services, then the services of the edition.
    let (health, health_service) = tonic_health::server::health_reporter();
    let mut routes = RoutesBuilder::default();
    routes.add_service(health_service);
    routes.add_service(tokens::server(providers.clone()));
    routes.add_service(accounts::account_server(ctx.clone()));
    routes.add_service(accounts::profile_server(ctx.clone()));
    routes.add_service(invitations::server(ctx));
    edition.add_services(&mut routes, &pool, &gate);

    // Put gRPC-Web on the gRPC routes only, then add the browser routes of the sign-in.
    let router = routes
        .routes()
        .into_axum_router()
        .layer(OperationLayer)
        .layer(GrpcWebLayer::new())
        .merge(providers::router(providers));

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
        .layer(layers::trace_layer())
        .layer(layers::cors_layer(&options.cors_origins))
        .add_routes(Routes::from(router))
        .serve_with_incoming_shutdown(TcpIncoming::from(listener), stop)
        .await?;
    Ok(())
}
