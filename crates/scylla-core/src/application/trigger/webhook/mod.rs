use crate::domain::ids::JobId;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

pub const DEFAULT_SIGNATURE_HEADER: &str = "X-Scylla-Signature-256";
pub const PING_EVENT: &str = "ping";

#[derive(Debug)]
pub enum IngestOutcome {
    Fired(JobId),
    Duplicate,
    Ping,
}

/// The lowercase hex digest when `signature`, "sha256=<hex>" or "<hex>" with the prefix and the
/// hex in any case, is the HMAC-SHA256 of `raw_body` under `secret`. The compare is constant-time.
#[must_use]
pub fn verified_digest(secret: &str, raw_body: &[u8], signature: &str) -> Option<String> {
    let signature = signature.trim();
    let hex_digest = match signature.get(..7) {
        Some(prefix) if prefix.eq_ignore_ascii_case("sha256=") => &signature[7..],
        _ => signature,
    };
    let digest = hex::decode(hex_digest.trim()).ok()?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).ok()?;
    mac.update(raw_body);
    mac.verify_slice(&digest).ok()?;
    Some(hex::encode(digest))
}

#[cfg(test)]
mod tests;
