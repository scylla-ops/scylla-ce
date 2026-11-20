//! The JSON Schema of the pipeline files, for the editors of the users: the public route
//! `GET /schemas/pipeline-v1.json`.

use std::sync::Arc;

use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use scylla_domain::projects::pipeline_files::json_schema;

/// The path of the schema.
const PATH: &str = "/schemas/pipeline-v1.json";

/// Makes the route of the schema. Its `$id` is its URL on the public API.
pub(super) fn router(api_url: &str) -> Router {
    let mut schema = json_schema();
    if let Some(object) = schema.as_object_mut() {
        let id = format!("{}{PATH}", api_url.trim_end_matches('/'));
        object.insert("$id".to_owned(), id.into());
    }
    let body: Arc<str> = schema.to_string().into();
    Router::new().route(PATH, get(serve)).with_state(body)
}

/// Serves the schema.
async fn serve(State(body): State<Arc<str>>) -> impl IntoResponse {
    (
        [
            (CONTENT_TYPE, "application/schema+json"),
            (CACHE_CONTROL, "public, max-age=3600"),
        ],
        body.to_string(),
    )
}
