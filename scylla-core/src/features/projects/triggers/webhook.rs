//! The public route of the webhooks: `POST /webhooks/{id}`. It has no token: the HMAC-SHA256 of
//! the body with the secret of the trigger is the credential, and the route checks it before
//! any write.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use hmac::{Hmac, KeyInit, Mac};
use scylla_domain::projects::triggers::TriggerId;
use serde_json::{json, Value};
use sha2::Sha256;

use super::{fire, service};
use crate::features::integrations::IntegrationContext;

/// The largest body, in bytes.
const MAX_BODY: usize = 1 << 20;

/// The most requests to one trigger in one window.
const RATE: u32 = 60;

/// The window of the rate limit.
const WINDOW: Duration = Duration::from_secs(60);

/// The headers of the signature, in order.
const SIGNATURES: [&str; 2] = ["x-scylla-signature-256", "x-hub-signature-256"];

/// The headers of the key of a delivery, in order.
const DELIVERIES: [&str; 2] = ["x-scylla-delivery", "x-github-delivery"];

/// What the route uses.
#[derive(Clone)]
pub(crate) struct Webhooks {
    /// What the Git integration shares: the pool, the hooks, the open streams of the agents, the
    /// key of the secrets, and the commit of a fire of a file pipeline.
    vcs: IntegrationContext,
    /// The requests of each trigger in the current window.
    rates: Arc<Mutex<HashMap<TriggerId, (Instant, u32)>>>,
}

impl Webhooks {
    /// Makes what the route uses, with no request counted.
    pub(crate) fn new(vcs: &IntegrationContext) -> Self {
        Self {
            vcs: vcs.clone(),
            rates: Arc::default(),
        }
    }
}

/// Makes the route, with its limit of the body.
pub(crate) fn router(webhooks: Webhooks) -> Router {
    Router::new()
        .route("/webhooks/{id}", post(receive))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state(webhooks)
}

/// Handles one delivery: 202 with the job, 200 for a ping or a second delivery, 401 for a bad
/// signature, 404 for an unknown or stopped trigger, 422 for a body with no value at a pointer,
/// 429 over the rate limit.
async fn receive(
    State(webhooks): State<Webhooks>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Find the trigger and its secret, under the rate limit.
    let Ok(id) = TriggerId::parse(&id) else {
        return answer(StatusCode::NOT_FOUND, json!({ "error": "unknown trigger" }));
    };
    if !webhooks.allow(id) {
        return answer(
            StatusCode::TOO_MANY_REQUESTS,
            json!({ "error": "too many requests" }),
        );
    }
    let (pool, sealer) = (&webhooks.vcs.pool, webhooks.vcs.sealer.as_deref());
    let secret = match service::webhook_secret(pool, sealer, id).await {
        Ok(Some(secret)) => secret,
        Ok(None) => return answer(StatusCode::NOT_FOUND, json!({ "error": "unknown trigger" })),
        Err(error) => return failure(error),
    };

    // Check the signature before any write.
    let Some(signature) = signature(&headers) else {
        return answer(
            StatusCode::UNAUTHORIZED,
            json!({ "error": "bad signature" }),
        );
    };
    let mut mac = Hmac::<Sha256>::new_from_slice(&secret).expect("HMAC takes each key length");
    mac.update(&body);
    if mac.verify_slice(&signature).is_err() {
        return answer(
            StatusCode::UNAUTHORIZED,
            json!({ "error": "bad signature" }),
        );
    }

    // Answer a ping, and a second delivery of one key.
    if header(&headers, "x-github-event") == Some("ping") {
        return answer(StatusCode::OK, json!({ "ping": true }));
    }
    let key = DELIVERIES
        .iter()
        .find_map(|name| header(&headers, name))
        .map(str::to_owned)
        .unwrap_or_else(|| hex::encode(&signature));
    match service::deliver(pool, id, &key).await {
        Ok(true) => {}
        Ok(false) => return answer(StatusCode::OK, json!({ "duplicate": true })),
        Err(error) => return failure(error),
    }

    // Fire the trigger. A failure forgets the key, so a retry of the sender fires.
    let payload: Option<Value> = serde_json::from_slice(&body).ok();
    let result = match service::get(pool, id).await {
        Ok(Some(trigger)) => fire::fire(&webhooks.vcs, &trigger, payload.as_ref()).await,
        Ok(None) => Err(fire::FireError::Failed(crate::Error::NotFound("trigger"))),
        Err(error) => Err(fire::FireError::Failed(error)),
    };
    match result {
        Ok(job) => answer(
            StatusCode::ACCEPTED,
            json!({ "job_id": job.id.to_string() }),
        ),
        Err(error) => {
            if let Err(forget) = service::forget(pool, id, &key).await {
                tracing::warn!(error = %forget, trigger = %id, "the key of a failed delivery stays");
            }
            match error {
                fire::FireError::Payload(reason) => {
                    answer(StatusCode::UNPROCESSABLE_ENTITY, json!({ "error": reason }))
                }
                fire::FireError::Failed(error) => failure(error),
            }
        }
    }
}

impl Webhooks {
    /// Counts a request of a trigger, and returns `false` over the limit of the window.
    fn allow(&self, id: TriggerId) -> bool {
        let mut rates = self.rates.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        if rates.len() > 10_000 {
            rates.retain(|_, (start, _)| now.duration_since(*start) < WINDOW);
        }
        let (start, count) = rates.entry(id).or_insert((now, 0));
        if now.duration_since(*start) >= WINDOW {
            (*start, *count) = (now, 0);
        }
        *count += 1;
        *count <= RATE
    }
}

/// Reads the signature of a request: hex, with or without the prefix `sha256=`.
fn signature(headers: &HeaderMap) -> Option<Vec<u8>> {
    let text = SIGNATURES.iter().find_map(|name| header(headers, name))?;
    hex::decode(text.trim().strip_prefix("sha256=").unwrap_or(text.trim())).ok()
}

/// Returns the text of a header.
fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// Makes a JSON answer.
fn answer(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

/// Logs a failure of the server and makes its answer, with no detail.
fn failure(error: crate::Error) -> Response {
    tracing::error!(event = "request.failed", %error, "a webhook failed");
    answer(
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({ "error": "internal error" }),
    )
}
