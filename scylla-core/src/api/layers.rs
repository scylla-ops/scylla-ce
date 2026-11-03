//! The layers of the server: CORS, and the stop signal.

use std::time::Duration;

use tonic::codegen::http::{HeaderName, Method};
use tower_http::cors::{AllowOrigin, CorsLayer};

use super::request_log::REQUEST_ID;

/// The headers that a gRPC-Web client sends.
const CORS_REQUEST_HEADERS: [&str; 5] = [
    "authorization",
    "content-type",
    "grpc-timeout",
    "x-grpc-web",
    "x-user-agent",
];

/// The headers that a gRPC-Web client must read.
const CORS_RESPONSE_HEADERS: [&str; 4] = [
    "grpc-status",
    "grpc-message",
    "grpc-status-details-bin",
    REQUEST_ID,
];

/// Lets only the listed web origins call the API from a browser.
pub(super) fn cors_layer(origins: &[String]) -> CorsLayer {
    let origins = origins.iter().filter_map(|origin| origin.parse().ok());
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_credentials(true)
        .allow_methods([Method::POST])
        .allow_headers(CORS_REQUEST_HEADERS.map(HeaderName::from_static))
        .expose_headers(CORS_RESPONSE_HEADERS.map(HeaderName::from_static))
        .max_age(Duration::from_secs(24 * 60 * 60))
}

/// Completes on SIGTERM (Kubernetes, Docker) or on Ctrl+C.
pub(super) async fn stop_signal() {
    // Wait for Ctrl+C, or forever if the process cannot listen for it.
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };

    // Wait for SIGTERM on Unix. Other systems have no SIGTERM.
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

    // Complete at the first of the two.
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}
