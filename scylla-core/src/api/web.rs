//! The web app on the address of the server: the files of a folder, or the files in the binary. A
//! path that is a file gives the file, and another path gives `index.html`, so the routes of the
//! web app load in the browser.

use std::path::Path;

use axum::extract::Request;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{from_fn, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get_service;
use axum::Router;
use scylla_extension::WebFiles;
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};

/// The cache of a file in `assets/`: the build puts a hash in its name.
const HASHED: &str = "public, max-age=31536000, immutable";
/// The cache of a page: it names the hashed files, so the browser must ask again.
const ASK_AGAIN: &str = "no-cache";

/// Makes the service of the web app: the folder of the settings first, then the files of the
/// binary. Returns nothing if there is no web app.
pub(super) fn app(dir: Option<&Path>, files: Option<WebFiles>) -> Option<Router> {
    let router = if let Some(dir) = dir {
        tracing::info!(dir = %dir.display(), "serving the web app of a folder");
        let index = ServeFile::new(dir.join("index.html"));
        Router::new().fallback_service(get_service(ServeDir::new(dir).fallback(index)))
    } else if let Some(files) = files.filter(|files| files("index.html").is_some()) {
        tracing::info!("serving the web app of the binary");
        Router::new().fallback(move |request: Request| async move { in_binary(files, &request) })
    } else {
        tracing::warn!("this binary has no web app: serving the API only");
        return None;
    };
    let router = router
        .layer(from_fn(cache))
        .layer(CompressionLayer::new())
        .layer(from_fn(answer_grpc));
    Some(router)
}

/// Gives a file of the binary, or `index.html`.
fn in_binary(files: WebFiles, request: &Request) -> Response {
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let path = request.uri().path().trim_start_matches('/');
    let Some(file) = files(path).or_else(|| files("index.html")) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let etag = format!("\"{}\"", hex::encode(&file.metadata.sha256_hash()[..16]));
    let known = request
        .headers()
        .get(IF_NONE_MATCH)
        .is_some_and(|value| value.as_bytes() == etag.as_bytes());
    if known {
        return (StatusCode::NOT_MODIFIED, [(ETAG, etag)]).into_response();
    }
    let kind = file.metadata.mimetype().to_owned();
    ([(CONTENT_TYPE, kind), (ETAG, etag)], file.data).into_response()
}

/// Sets the cache of an answer. A missing file in `assets/` gives a page, with the cache of a page.
async fn cache(request: Request, next: Next) -> Response {
    let hashed = request.uri().path().starts_with("/assets/");
    let mut response = next.run(request).await;
    let page = kind_of(response.headers()).is_some_and(|kind| kind.starts_with("text/html"));
    let policy = if hashed && !page { HASHED } else { ASK_AGAIN };
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static(policy));
    response
}

/// Answers a gRPC call of an unknown service with UNIMPLEMENTED, and not with a page.
async fn answer_grpc(request: Request, next: Next) -> Response {
    let Some(kind) = kind_of(request.headers()).filter(|kind| kind.starts_with("application/grpc"))
    else {
        return next.run(request).await;
    };
    let echo = kind
        .starts_with("application/grpc-web")
        .then(|| HeaderValue::from_str(kind).ok())
        .flatten();
    let mut response: Response = tonic::Status::unimplemented("").into_http();
    if let Some(echo) = echo {
        response.headers_mut().insert(CONTENT_TYPE, echo);
    }
    response
}

/// Returns the content type of a request or of an answer.
fn kind_of(headers: &HeaderMap) -> Option<&str> {
    headers.get(CONTENT_TYPE)?.to_str().ok()
}
