pub mod commands;

pub use commands::{Delivery, IngestWebhook};

use crate::application::SecretCipher;
use crate::application::trigger::delivery::TriggerDeliveryRepository;
use crate::application::trigger::fire::TriggerFiring;
use crate::application::trigger::repository::TriggerRepository;
use crate::domain::ids::JobId;
use derive_more::Constructor;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use std::sync::Arc;

pub const DEFAULT_SIGNATURE_HEADER: &str = "X-Scylla-Signature-256";
pub const PING_EVENT: &str = "ping";

#[derive(Debug)]
pub enum IngestOutcome {
    Fired(JobId),
    Duplicate,
    Ping,
}

/// The webhook ingress's stage runners, one block per action in `commands.rs`. `IngestWebhook`
/// is `Public`: the signature is the credential, and it is verified before any write, so an
/// unauthenticated caller cannot pollute the dedupe table.
#[derive(Constructor)]
pub struct WebhookIngressUseCases {
    pub(super) trigger_repo: Arc<dyn TriggerRepository>,
    pub(super) delivery_repo: Arc<dyn TriggerDeliveryRepository>,
    pub(super) cipher: Arc<dyn SecretCipher>,
    pub(super) firing: Arc<dyn TriggerFiring>,
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
