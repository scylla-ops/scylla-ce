use crate::application::{IngestOutcome, IngestWebhook, WebhookIngressUseCases};
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::TriggerId;
use crate::rest::adapter::run_public;
use axum::{
    Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use scylla_extension::Actions;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::error;

const DELIVERY_HEADERS: [&str; 2] = ["X-Scylla-Delivery", "X-GitHub-Delivery"];
const EVENT_HEADERS: [&str; 2] = ["X-Scylla-Event", "X-GitHub-Event"];

#[derive(Clone)]
struct Webhooks {
    actions: Arc<Actions>,
    ingress: Arc<WebhookIngressUseCases>,
}

pub fn router(actions: Arc<Actions>, ingress: Arc<WebhookIngressUseCases>) -> Router {
    Router::new()
        .route("/webhooks/{trigger_id}", post(handle))
        .with_state(Webhooks { actions, ingress })
}

async fn handle(
    State(webhooks): State<Webhooks>,
    Path(trigger_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, &'static str) {
    let command = ingest_webhook(&trigger_id, &headers, body);
    reply(&run_public(&webhooks.actions, &*webhooks.ingress, command).await)
}

fn reply(result: &DomainResult<IngestOutcome>) -> (StatusCode, &'static str) {
    match result {
        // A missing agent is not an error: the run is minted and pending.
        Ok(IngestOutcome::Fired(_)) => (StatusCode::ACCEPTED, "accepted"),
        Ok(IngestOutcome::Duplicate) => (StatusCode::OK, "duplicate"),
        Ok(IngestOutcome::Ping) => (StatusCode::OK, "pong"),
        Err(DomainError::NotFound(_)) => (StatusCode::NOT_FOUND, "not found"),
        Err(DomainError::Unauthorized(_)) => (StatusCode::UNAUTHORIZED, "invalid signature"),
        Err(DomainError::Validation(_)) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid payload"),
        Err(e) => {
            error!(error = %e, "webhook delivery failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error")
        }
    }
}

/// Each header keeps its first value, if that value is text, under its lowercase name.
fn ingest_webhook(trigger_id: &str, headers: &HeaderMap, body: Bytes) -> IngestWebhook {
    let first_header = |names: &[&str]| {
        names.iter().find_map(|h| {
            headers
                .get(*h)
                .and_then(|v| v.to_str().ok())
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
    };
    let values: HashMap<_, _> = headers
        .keys()
        .filter_map(|name| {
            let value = headers.get(name)?.to_str().ok()?;
            Some((name.as_str().to_owned(), value.to_owned()))
        })
        .collect();
    IngestWebhook {
        trigger_id: TriggerId::new(trigger_id),
        headers: values,
        delivery_id: first_header(&DELIVERY_HEADERS),
        event: first_header(&EVENT_HEADERS),
        body: body.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::JobId;
    use axum::http::HeaderValue;

    #[test]
    fn a_custom_header_is_kept_under_its_lowercase_name_with_its_first_value() {
        let mut headers = HeaderMap::new();
        headers.append("X-Custom-Signature", HeaderValue::from_static("sha256=abc"));
        headers.append(
            "X-Custom-Signature",
            HeaderValue::from_static("sha256=later"),
        );
        headers.insert("X-GitHub-Delivery", HeaderValue::from_static("d-1"));
        headers.insert("x-scylla-event", HeaderValue::from_static("push"));
        headers.insert("X-Binary", HeaderValue::from_bytes(&[0xFF]).unwrap());

        let cmd = ingest_webhook("t-1", &headers, Bytes::from_static(b"{}"));

        assert_eq!(cmd.trigger_id, TriggerId::new("t-1"));
        assert_eq!(
            cmd.headers.get("x-custom-signature").map(String::as_str),
            Some("sha256=abc")
        );
        assert!(!cmd.headers.contains_key("X-Custom-Signature"));
        assert!(!cmd.headers.contains_key("x-binary"));
        assert_eq!(cmd.delivery_id.as_deref(), Some("d-1"));
        assert_eq!(cmd.event.as_deref(), Some("push"));
        assert_eq!(cmd.body, b"{}".to_vec());
    }

    #[test]
    fn a_scylla_delivery_header_wins_over_the_github_one_and_an_empty_one_is_skipped() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Scylla-Delivery", HeaderValue::from_static(""));
        headers.insert("X-GitHub-Delivery", HeaderValue::from_static("gh-1"));

        let cmd = ingest_webhook("t-1", &headers, Bytes::new());

        assert_eq!(cmd.delivery_id.as_deref(), Some("gh-1"));
        assert_eq!(cmd.event, None);
    }

    #[test]
    fn each_outcome_and_error_has_its_status() {
        let cases = [
            (
                Ok(IngestOutcome::Fired(JobId::new("j"))),
                StatusCode::ACCEPTED,
            ),
            (Ok(IngestOutcome::Duplicate), StatusCode::OK),
            (Ok(IngestOutcome::Ping), StatusCode::OK),
            (
                Err(DomainError::not_found("Trigger", "t")),
                StatusCode::NOT_FOUND,
            ),
            (
                Err(DomainError::unauthorized("bad")),
                StatusCode::UNAUTHORIZED,
            ),
            (
                Err(DomainError::validation("nul")),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                Err(DomainError::forbidden("no")),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                Err(DomainError::internal("db")),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];
        for (result, status) in cases {
            assert_eq!(reply(&result).0, status);
        }
    }
}
