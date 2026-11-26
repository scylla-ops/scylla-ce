//! The route `GET /images/{id}`: the PNG of an uploaded image, or the SVG of a glyph avatar. The
//! id is the key: the route needs no token, so a page shows the image with an `img` element. The
//! answer has a fixed content type, no sniffing, a policy that runs nothing, and a long cache, as
//! an image never changes.

use axum::extract::{Path, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use scylla_domain::platform::images::{Image, ImageId};
use sqlx::PgPool;

use super::service;

/// The cache of an image: a new image has a new id.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// The security policy of an image: it loads and runs nothing.
const NOTHING: &str = "default-src 'none'; sandbox";

/// Makes the route of the images.
pub(crate) fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/images/{id}", get(image))
        .with_state(pool)
}

/// Gives the PNG or the SVG of an image. A text that is not an id and an unknown id give `404`.
async fn image(State(pool): State<PgPool>, Path(id): Path<String>) -> Response {
    let Ok(id) = ImageId::parse(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match service::find(&pool, id).await {
        Ok(Some((content_type, data))) => {
            let content_type = match content_type.as_str() {
                Image::SVG_CONTENT_TYPE => Image::SVG_CONTENT_TYPE,
                _ => Image::CONTENT_TYPE,
            };
            let headers = [
                (CONTENT_TYPE, content_type),
                (X_CONTENT_TYPE_OPTIONS, "nosniff"),
                (CONTENT_SECURITY_POLICY, NOTHING),
                (CACHE_CONTROL, IMMUTABLE),
            ];
            (headers, data).into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::error!(%error, image = %id, "the read of an image failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
