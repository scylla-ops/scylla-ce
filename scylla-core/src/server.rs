//! The gRPC server: gate, operations, gRPC-Web, CORS, TLS, health, tracing, timeout and graceful
//! shutdown.

use std::fs;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use scylla_extension::{Edition, Gate};
use sqlx::PgPool;
use tokio::net::TcpListener;
use tonic::codegen::http::{HeaderName, Method};
use tonic::service::RoutesBuilder;
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Identity, Server, ServerTlsConfig};
use tonic_health::ServingStatus;
use tonic_web::GrpcWebLayer;
use tower_http::classify::{GrpcCode, GrpcErrorsAsFailures, SharedClassifier};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::access::{Access, OperationLayer};
use crate::features::tokens;
use crate::features::users::OidcVerifier;
use crate::{db, Config, Error};

/// The headers that a gRPC-Web client sends.
const CORS_REQUEST_HEADERS: [&str; 5] = [
    "authorization",
    "content-type",
    "grpc-timeout",
    "x-grpc-web",
    "x-user-agent",
];

/// The headers that a gRPC-Web client must read.
const CORS_RESPONSE_HEADERS: [&str; 3] = ["grpc-status", "grpc-message", "grpc-status-details-bin"];

/// The options of `serve`.
#[derive(Clone)]
pub struct ServeOptions {
    /// The time after which the server stops a request.
    pub request_timeout: Duration,
    /// The certificate and key of the server. `None` serves without TLS.
    pub tls: Option<Identity>,
    /// The web origins that can call the API with gRPC-Web.
    pub cors_origins: Vec<String>,
    /// The check of the ID tokens of the users. `None` refuses `ExchangeIdToken`.
    pub oidc: Option<Arc<OidcVerifier>>,
}

impl Default for ServeOptions {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(15),
            tls: None,
            cors_origins: Vec::new(),
            oidc: None,
        }
    }
}

/// Checks the schema, then serves on the address of the config until SIGTERM or Ctrl+C.
pub async fn run(config: &Config, edition: Edition) -> Result<(), Error> {
    let tls = match (&config.tls_cert, &config.tls_key) {
        (Some(cert), Some(key)) => Some(Identity::from_pem(fs::read(cert)?, fs::read(key)?)),
        _ => None,
    };
    let oidc = match (&config.oidc_issuer, &config.oidc_audience) {
        (Some(issuer), Some(audience)) => Some(Arc::new(
            OidcVerifier::discover(issuer, audience)
                .await
                .map_err(|reason| Error::Config {
                    name: "SCYLLA_OIDC_ISSUER",
                    reason,
                })?,
        )),
        _ => None,
    };
    let pool = db::connect(config).await?;
    db::check_migrated(&pool, edition.migrators()).await?;
    let access = Arc::new(Access::from_config(config, pool.clone())?);
    let listener = TcpListener::bind(config.bind_addr).await?;
    tracing::info!(
        edition = edition.name(),
        addr = %listener.local_addr()?,
        tls = tls.is_some(),
        oidc = oidc.is_some(),
        "listening"
    );
    let options = ServeOptions {
        request_timeout: config.request_timeout,
        tls,
        cors_origins: config.cors_origins.clone(),
        oidc,
    };
    serve(
        listener,
        pool.clone(),
        edition,
        access,
        options,
        stop_signal(),
    )
    .await?;
    pool.close().await;
    tracing::info!("stopped");
    Ok(())
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
    let gate: Arc<dyn Gate> = access;
    let (health, health_service) = tonic_health::server::health_reporter();
    let mut routes = RoutesBuilder::default();
    routes.add_service(health_service);
    routes.add_service(tokens::server(
        pool.clone(),
        Arc::clone(&gate),
        options.oidc,
    ));
    edition.add_services(&mut routes, &pool, &gate);

    let stop = async move {
        stop.await;
        health
            .set_service_status("", ServingStatus::NotServing)
            .await;
        tracing::info!("stopping, waiting for the running requests");
    };
    let mut server = Server::builder().accept_http1(true);
    if let Some(identity) = options.tls {
        server = server.tls_config(ServerTlsConfig::new().identity(identity))?;
    }
    server
        .timeout(options.request_timeout)
        .layer(trace_layer())
        .layer(cors_layer(&options.cors_origins))
        .layer(GrpcWebLayer::new())
        .layer(OperationLayer)
        .add_routes(routes.routes())
        .serve_with_incoming_shutdown(TcpIncoming::from(listener), stop)
        .await?;
    Ok(())
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
        .with_success(GrpcCode::PermissionDenied)
        .with_success(GrpcCode::NotFound)
        .with_success(GrpcCode::FailedPrecondition);
    TraceLayer::new(SharedClassifier::new(classifier))
        .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
        .on_response(DefaultOnResponse::new().level(Level::INFO))
}

/// Lets only the listed web origins call the API from a browser.
fn cors_layer(origins: &[String]) -> CorsLayer {
    let origins = origins.iter().filter_map(|origin| origin.parse().ok());
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::POST])
        .allow_headers(CORS_REQUEST_HEADERS.map(HeaderName::from_static))
        .expose_headers(CORS_RESPONSE_HEADERS.map(HeaderName::from_static))
        .max_age(Duration::from_secs(24 * 60 * 60))
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
