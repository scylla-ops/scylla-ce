//! The HTTP calls to an identity provider, with their errors.

use scylla_extension::AccessError;
use serde::de::DeserializeOwned;

use crate::features::identity::accounts::refused;
use crate::Error;

/// Sends a request and reads its JSON answer. A failure is `UNAVAILABLE`.
pub(super) async fn send<T: DeserializeOwned>(
    request: reqwest::RequestBuilder,
) -> Result<T, Error> {
    let response = request
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(failed)?;
    response.json().await.map_err(badly)
}

/// Sends a code exchange and reads its token. A provider names its refusal in the field `error`,
/// and GitHub does it with a status of success.
pub(super) async fn send_exchange<T: DeserializeOwned>(
    request: reqwest::RequestBuilder,
) -> Result<T, Error> {
    let response = request.send().await.map_err(failed)?;
    let status = response.error_for_status_ref().map(|_| ());
    let body: serde_json::Value = match response.json().await {
        Ok(body) => body,
        Err(error) => return Err(status.err().map_or_else(|| badly(error), failed)),
    };
    match body.get("error").and_then(serde_json::Value::as_str) {
        Some("redirect_uri_mismatch") => {
            let message = "the identity provider did not accept the redirect URL";
            return Err(refused("REDIRECT_URI_MISMATCH", message));
        }
        Some(_) => {
            let message = "the identity provider refused the code";
            return Err(refused("PROVIDER_REFUSED", message));
        }
        None => status.map_err(failed)?,
    }
    serde_json::from_value(body).map_err(|_| unavailable("the identity provider answered badly"))
}

/// The error of an identity provider that does not answer well.
pub(super) fn unavailable(reason: &str) -> Error {
    AccessError::Unavailable(reason.to_owned()).into()
}

/// The error of a call that failed.
fn failed(error: reqwest::Error) -> Error {
    unavailable(&format!("the identity provider failed: {error}"))
}

/// The error of an answer that is not the expected JSON.
fn badly(error: reqwest::Error) -> Error {
    unavailable(&format!("the identity provider answered badly: {error}"))
}
