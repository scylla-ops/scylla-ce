mod cache;
mod disk;
mod embedded;
mod grpc;

use crate::config::UiConfig;
use axum::Router;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::from_fn;
use axum::response::{IntoResponse, Response};
use std::convert::Infallible;
use tower::ServiceBuilder;
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

pub fn attach(app: Router, ui: &UiConfig) -> Router {
    let guard = || from_fn(grpc::guard);

    if !ui.enabled {
        tracing::info!("web UI disabled: serving the API only");
        let service = ServiceBuilder::new()
            .layer(guard())
            .service(tower::service_fn(disabled));
        return app.fallback_service(service);
    }

    // Compression here, not on the whole router: tower-http's default predicate exempts gRPC-Web, so a global layer would rewrite those bodies.
    // The HTTP trace classifier replaces the gRPC one, which reads `grpc-status` and calls every 404 a success.
    let layers = || {
        ServiceBuilder::new()
            .layer(TraceLayer::new_for_http())
            .layer(CompressionLayer::new().gzip(true).br(true))
            .layer(guard())
    };

    if let Some(dir) = ui.dir.as_deref() {
        tracing::info!(dir = %dir.display(), "serving web UI from disk");
        return app.fallback_service(layers().service(disk::service(dir)));
    }

    match embedded::count() {
        0 => tracing::warn!(
            "no web UI embedded in this binary: serving the API only. Build the web UI \
             (`just ui-build`) and rebuild, or point [ui].dir at a dist/."
        ),
        assets => tracing::info!(assets, "serving embedded web UI"),
    }
    app.fallback_service(layers().service(tower::service_fn(embedded::serve)))
}

async fn disabled(_request: Request) -> Result<Response, Infallible> {
    Ok((
        StatusCode::NOT_FOUND,
        "web UI is disabled on this instance\n",
    )
        .into_response())
}
