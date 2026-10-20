use super::cache;
use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rust_embed::EmbeddedFile;
use std::convert::Infallible;

/// `allow_missing` keeps a fresh checkout compiling with no `pnpm build`.
/// Debug builds read from disk at request time; release builds embed.
#[derive(rust_embed::RustEmbed)]
#[folder = "../../web/dist/"]
#[allow_missing = true]
struct Assets;

pub(super) fn count() -> usize {
    Assets::iter().count()
}

pub(super) async fn serve(request: Request) -> Result<Response, Infallible> {
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        return Ok(StatusCode::METHOD_NOT_ALLOWED.into_response());
    }

    let path = request.uri().path().trim_start_matches('/');
    let (asset, immutable) = match Assets::get(path) {
        Some(asset) => (asset, cache::is_immutable(path)),
        None => match Assets::get("index.html") {
            Some(index) => (index, false),
            None => return Ok(not_bundled()),
        },
    };

    let etag = etag(&asset);
    if request
        .headers()
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == etag)
    {
        return Ok(StatusCode::NOT_MODIFIED.into_response());
    }

    let mut response = (
        [
            (header::CONTENT_TYPE, asset.metadata.mimetype().to_owned()),
            (header::ETAG, etag),
        ],
        asset.data,
    )
        .into_response();
    cache::apply(&mut response, immutable);
    Ok(response)
}

fn etag(asset: &EmbeddedFile) -> String {
    format!("\"{}\"", hex::encode(&asset.metadata.sha256_hash()[..16]))
}

fn not_bundled() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "No web UI is bundled in this build of Scylla.\n\
         Build the web UI (`just ui-build`) and rebuild, or set [ui].dir.\n",
    )
        .into_response()
}
