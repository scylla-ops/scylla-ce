//! The answers of the routes of the webhooks of the providers, and the headers that they read.

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::ProjectId;
use serde_json::{json, Value};

use crate::infra::vcs::SignatureError;
use crate::Error;

/// Returns the text of a header.
pub(super) fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// Logs a refused signature, and answers 401.
pub(super) fn rejected(
    provider: VcsProvider,
    error: SignatureError,
    project: Option<ProjectId>,
) -> Response {
    let project = project
        .map(|project| project.to_string())
        .unwrap_or_default();
    tracing::warn!(
        event = "vcs.webhook_rejected",
        provider = provider.as_str(),
        project,
        reason = %error,
    );
    answer(
        StatusCode::UNAUTHORIZED,
        json!({ "error": "bad signature" }),
    )
}

/// Answers 404 for an unknown App or webhook.
pub(super) fn unknown() -> Response {
    answer(StatusCode::NOT_FOUND, json!({ "error": "unknown webhook" }))
}

/// Answers 400 with a reason.
pub(super) fn bad_request(reason: &str) -> Response {
    answer(StatusCode::BAD_REQUEST, json!({ "error": reason }))
}

/// Makes a JSON answer.
pub(super) fn answer(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

/// Logs a failure of the server and makes its answer, with no detail.
pub(super) fn failure(error: Error) -> Response {
    tracing::error!(event = "request.failed", %error, "a webhook of a provider failed");
    answer(
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({ "error": "internal error" }),
    )
}
