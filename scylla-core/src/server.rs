//! The gRPC server: authentication chain, TLS, health, tracing, timeout and graceful shutdown.

use std::fs;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use scylla_extension::{Authenticator, Authenticators, Edition};
use sqlx::PgPool;
use tokio::net::TcpListener;
use tonic::service::RoutesBuilder;
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Identity, Server, ServerTlsConfig};
use tonic_health::ServingStatus;
use tower_http::classify::{GrpcCode, GrpcErrorsAsFailures, SharedClassifier};
use tower_http::trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::features::actors::TrustedHeader;
use crate::{db, Config, Error};

/// The options of `serve`.
#[derive(Clone, Debug)]
pub struct ServeOptions {
    /// The time after which the server stops a request.
    pub request_timeout: Duration,
    /// Adds the dev authenticator `TrustedHeader` after those of the edition.
    pub trust_actor_header: bool,
    /// The certificate and key of the server. `None` serves without TLS.
    pub tls: Option<Identity>,
}

/// Checks the schema, then serves on the address of the config until SIGTERM or Ctrl+C.
pub async fn run(config: &Config, edition: Edition) -> Result<(), Error> {
    let tls = match (&config.tls_cert, &config.tls_key) {
        (Some(cert), Some(key)) => Some(Identity::from_pem(fs::read(cert)?, fs::read(key)?)),
        _ => None,
    };
    let pool = db::connect(config).await?;
    db::check_migrated(&pool, edition.migrators()).await?;
    let listener = TcpListener::bind(config.bind_addr).await?;
    tracing::info!(
        edition = edition.name(),
        addr = %listener.local_addr()?,
        tls = tls.is_some(),
        "listening"
    );
    let options = ServeOptions {
        request_timeout: config.request_timeout,
        trust_actor_header: config.trust_actor_header,
        tls,
    };
    serve(listener, pool.clone(), edition, options, stop_signal()).await?;
    pool.close().await;
    tracing::info!("stopped");
    Ok(())
}

/// Serves on a bound listener until `stop`, then waits for the running requests.
pub async fn serve(
    listener: TcpListener,
    pool: PgPool,
    edition: Edition,
    options: ServeOptions,
    stop: impl Future<Output = ()>,
) -> Result<(), Error> {
    let authenticators = Arc::new(authenticators(&edition, options.trust_actor_header));
    let (health, health_service) = tonic_health::server::health_reporter();
    let mut routes = RoutesBuilder::default();
    routes.add_service(health_service);
    edition.add_services(&mut routes, &pool, &authenticators);

    let stop = async move {
        stop.await;
        health
            .set_service_status("", ServingStatus::NotServing)
            .await;
        tracing::info!("stopping, waiting for the running requests");
    };
    let mut server = Server::builder();
    if let Some(identity) = options.tls {
        server = server.tls_config(ServerTlsConfig::new().identity(identity))?;
    }
    server
        .timeout(options.request_timeout)
        .layer(trace_layer())
        .add_routes(routes.routes())
        .serve_with_incoming_shutdown(TcpIncoming::from(listener), stop)
        .await?;
    Ok(())
}

/// Makes the authenticator chain: those of the edition, then the dev header if enabled.
fn authenticators(edition: &Edition, trust_actor_header: bool) -> Authenticators {
    let mut chain = edition.authenticators().to_vec();
    if trust_actor_header {
        tracing::warn!("the x-scylla-actor header is trusted: do not use this in production");
        chain.push(Arc::new(TrustedHeader) as Arc<dyn Authenticator>);
    }
    if chain.is_empty() {
        tracing::warn!("no authenticator: each request that needs an actor is refused");
    }
    Authenticators::new(chain)
}

/// The trace layer type of the server.
type Trace = TraceLayer<
    SharedClassifier<GrpcErrorsAsFailures>,
    DefaultMakeSpan,
    DefaultOnRequest,
    DefaultOnResponse,
>;

/// Makes one span and one log line per request. Client errors log as responses, not failures.
fn trace_layer() -> Trace {
    let classifier = GrpcErrorsAsFailures::new()
        .with_success(GrpcCode::InvalidArgument)
        .with_success(GrpcCode::Unauthenticated)
        .with_success(GrpcCode::NotFound)
        .with_success(GrpcCode::FailedPrecondition);
    TraceLayer::new(SharedClassifier::new(classifier))
        .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
        .on_response(DefaultOnResponse::new().level(Level::INFO))
}

/// Completes on SIGTERM (Kubernetes, Docker) or on Ctrl+C.
async fn stop_signal() {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}
