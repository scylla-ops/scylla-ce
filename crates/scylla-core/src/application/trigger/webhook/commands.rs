//! The webhook delivery: the command and its dedupe key.

use super::IngestOutcome;
use crate::domain::ids::TriggerId;
use scylla_extension::{Access, Command, Describe, Draft};
use std::collections::HashMap;

/// No `Debug`: the headers carry the signature. `headers` is keyed by the lowercase name.
pub struct IngestWebhook {
    pub trigger_id: TriggerId,
    pub headers: HashMap<String, String>,
    pub delivery_id: Option<String>,
    pub event: Option<String>,
    pub body: Vec<u8>,
}

/// A ping is answered without a write; an event is recorded under its dedupe key, then fired.
/// The key is the delivery id, else the verified digest, so every form of one signature is one key.
pub enum Delivery {
    Ping,
    Event {
        trigger_id: TriggerId,
        key: String,
        delivery_id: Option<String>,
        payload: Option<serde_json::Value>,
    },
}

impl Describe for IngestWebhook {
    fn access(&self) -> Access {
        Access::Public
    }
}

impl Command for IngestWebhook {
    type Staged = Draft<Delivery>;
    type Committed = IngestOutcome;
}
