//! The errors of a call to a Git provider, from the status, the headers and the body of its
//! answer: the primary and the secondary limits of GitHub, and the limits of GitLab.

use reqwest::header::HeaderMap;
use reqwest::StatusCode;
use serde_json::Value;
use time::{Duration, OffsetDateTime};

/// The wait after a limit with no time in the headers.
const DEFAULT_WAIT: Duration = Duration::minutes(1);

/// The longest message of a refusal that an error keeps, in characters.
const MAX_MESSAGE: usize = 200;

/// A failed call to a Git provider.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    /// The provider limits the calls: call again after this time.
    #[error("the provider limits the calls until {0}")]
    RateLimited(OffsetDateTime),
    /// The credentials are not valid, have expired, or were revoked (401).
    #[error("the provider refused the credentials")]
    Unauthorized,
    /// The credentials do not give this call (403).
    #[error("the provider refused the call")]
    Forbidden,
    /// The object does not exist, or the credentials do not see it (404, 410).
    #[error("the provider found no such object")]
    NotFound,
    /// The provider refused the request, or its answer is not valid.
    #[error("the provider refused the request: {0}")]
    Invalid(String),
    /// A failure that can go away: a timeout, a lost connection or an error of the server.
    #[error("the provider failed: {0}")]
    Transient(String),
}

impl ProviderError {
    /// Returns the error of an answer that is not a success, at the time `now`.
    pub fn of_answer(
        status: StatusCode,
        headers: &HeaderMap,
        body: &[u8],
        now: OffsetDateTime,
    ) -> Self {
        if let Some(until) = limit(status, headers, body, now) {
            return Self::RateLimited(until);
        }
        match status.as_u16() {
            401 => Self::Unauthorized,
            403 => Self::Forbidden,
            404 | 410 => Self::NotFound,
            408 | 500..=599 => Self::Transient(format!("status {}", status.as_u16())),
            _ => Self::Invalid(message(status, body)),
        }
    }

    /// Returns the error of a call with no answer. The text has no URL.
    pub fn of_transport(error: reqwest::Error) -> Self {
        Self::Transient(error.without_url().to_string())
    }

    /// Returns `true` when a later call can work: a limit or a transient failure.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::RateLimited(_) | Self::Transient(_))
    }
}

/// Returns the end of a limit, if the answer is one: the seconds of `retry-after`, else the
/// reset time when no call remains, else one minute for a secondary limit or a 429.
fn limit(
    status: StatusCode,
    headers: &HeaderMap,
    body: &[u8],
    now: OffsetDateTime,
) -> Option<OffsetDateTime> {
    // Only a 403 or a 429 can be a limit.
    if !matches!(status.as_u16(), 403 | 429) {
        return None;
    }

    // Wait the seconds of `retry-after`.
    if let Some(seconds) = number(headers, "retry-after") {
        return Some(now + Duration::seconds(seconds.max(0)));
    }

    // Wait for the reset when no call remains.
    let remaining =
        number(headers, "x-ratelimit-remaining").or_else(|| number(headers, "ratelimit-remaining"));
    if remaining == Some(0) {
        let reset =
            number(headers, "x-ratelimit-reset").or_else(|| number(headers, "ratelimit-reset"));
        let at = reset.and_then(|epoch| OffsetDateTime::from_unix_timestamp(epoch).ok());
        return Some(at.filter(|at| *at > now).unwrap_or(now + DEFAULT_WAIT));
    }

    // Wait one minute after a secondary limit, or a 429 with no time.
    let secondary = message(status, body).to_lowercase().contains("rate limit");
    (status.as_u16() == 429 || secondary).then_some(now + DEFAULT_WAIT)
}

/// Reads a header as a whole number.
fn number(headers: &HeaderMap, name: &str) -> Option<i64> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

/// Returns the message of a refusal: `message`, `error_description` or `error` of a JSON body,
/// or the status.
fn message(status: StatusCode, body: &[u8]) -> String {
    let json: Option<Value> = serde_json::from_slice(body).ok();
    let text = json.as_ref().and_then(|json| {
        ["message", "error_description", "error"]
            .iter()
            .find_map(|key| json.get(*key))
            .map(|value| {
                value
                    .as_str()
                    .map_or_else(|| value.to_string(), str::to_owned)
            })
    });
    match text {
        Some(text) => text.chars().take(MAX_MESSAGE).collect(),
        None => format!("status {}", status.as_u16()),
    }
}
