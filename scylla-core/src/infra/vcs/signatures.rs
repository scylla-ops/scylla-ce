//! The signatures of the webhooks of the providers, checked in constant time: GitHub
//! `X-Hub-Signature-256`, and GitLab `webhook-signature` (Standard Webhooks) over
//! `id.timestamp.body` with a window of 5 minutes.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use time::{Duration, OffsetDateTime};

use super::Hidden;

/// The largest gap between the time of a GitLab delivery and now.
pub const GITLAB_WINDOW: Duration = Duration::minutes(5);

/// The prefix of a GitLab signing token.
const TOKEN_PREFIX: &str = "whsec_";

/// The number of random bytes in a GitLab signing token.
const TOKEN_BYTES: usize = 32;

/// Why a signature is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SignatureError {
    /// A header of the signature is missing.
    #[error("the signature is missing")]
    Missing,
    /// The signature does not match the body.
    #[error("the signature does not match")]
    Bad,
    /// The time of the delivery is out of the window.
    #[error("the delivery is out of the time window")]
    Old,
}

/// The headers of a GitLab delivery.
#[derive(Clone, Copy, Debug, Default)]
pub struct GitLabDelivery<'a> {
    /// `webhook-id`: the id of the message, the same in each retry.
    pub id: Option<&'a str>,
    /// `webhook-timestamp`: the time of the delivery, in seconds since the epoch.
    pub timestamp: Option<&'a str>,
    /// `webhook-signature`: signatures `v1,<base64>`, separated by spaces.
    pub signature: Option<&'a str>,
}

/// Checks `X-Hub-Signature-256: sha256=<hex>`: the HMAC-SHA256 of the body with the secret.
pub fn check_github(
    secret: &[u8],
    header: Option<&str>,
    body: &[u8],
) -> Result<(), SignatureError> {
    let header = header.ok_or(SignatureError::Missing)?;
    let hex_text = header
        .trim()
        .strip_prefix("sha256=")
        .ok_or(SignatureError::Bad)?;
    let signature = hex::decode(hex_text).map_err(|_| SignatureError::Bad)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC takes each key length");
    mac.update(body);
    mac.verify_slice(&signature)
        .map_err(|_| SignatureError::Bad)
}

/// Returns `sha256=<hex>`: the GitHub signature of a body, as a sender makes it.
pub fn sign_github(secret: &[u8], body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC takes each key length");
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

/// Checks a GitLab delivery: a time within 5 minutes of `now`, and one signature `v1,<base64>`
/// that is the HMAC-SHA256 of `id.timestamp.body` with the key of the signing token.
pub fn check_gitlab(
    signing_token: &Hidden,
    delivery: &GitLabDelivery<'_>,
    body: &[u8],
    now: OffsetDateTime,
) -> Result<(), SignatureError> {
    // Refuse a delivery with a missing header, or out of the window.
    let (Some(id), Some(timestamp), Some(signatures)) =
        (delivery.id, delivery.timestamp, delivery.signature)
    else {
        return Err(SignatureError::Missing);
    };
    let sent: i64 = timestamp.trim().parse().map_err(|_| SignatureError::Bad)?;
    if (now.unix_timestamp() - sent).abs() > GITLAB_WINDOW.whole_seconds() {
        return Err(SignatureError::Old);
    }

    // Accept one signature of the list that matches the HMAC.
    let mac = gitlab_mac(signing_token, id, timestamp, body).ok_or(SignatureError::Bad)?;
    let matches = signatures
        .split(' ')
        .filter_map(|signature| signature.strip_prefix("v1,"))
        .filter_map(|signature| STANDARD.decode(signature).ok())
        .any(|signature| mac.clone().verify_slice(&signature).is_ok());
    matches.then_some(()).ok_or(SignatureError::Bad)
}

/// Returns `v1,<base64>`: the GitLab signature of a delivery, as GitLab makes it.
pub fn sign_gitlab(signing_token: &Hidden, id: &str, timestamp: &str, body: &[u8]) -> String {
    let mac = gitlab_mac(signing_token, id, timestamp, body).expect("a signing token in base64");
    format!("v1,{}", STANDARD.encode(mac.finalize().into_bytes()))
}

/// Makes a new GitLab signing token: `whsec_` and 32 random bytes in base64.
pub fn gitlab_signing_token() -> Hidden {
    let mut key = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut key).expect("the system gives random bytes");
    Hidden::new(format!("{TOKEN_PREFIX}{}", STANDARD.encode(key)))
}

/// Returns the HMAC of `id.timestamp.body` with the key of a signing token, or `None` when the
/// token is not base64.
fn gitlab_mac(
    signing_token: &Hidden,
    id: &str,
    timestamp: &str,
    body: &[u8],
) -> Option<Hmac<Sha256>> {
    let token = signing_token.expose();
    let key = STANDARD
        .decode(token.strip_prefix(TOKEN_PREFIX).unwrap_or(token))
        .ok()?;
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).expect("HMAC takes each key length");
    let dot = b".".as_slice();
    for part in [id.as_bytes(), dot, timestamp.as_bytes(), dot, body] {
        mac.update(part);
    }
    Some(mac)
}
