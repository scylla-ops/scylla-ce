//! The webhook ingress's writes. One block per command, in the order it runs: the struct, its
//! access, its payload types, what `Prepare` builds, what `Persist` writes. An unknown, disabled
//! or non-webhook trigger is one opaque `NotFound`, also when it is deleted or disabled while the
//! delivery is in flight; a missing or wrong signature is `Unauthorized`, and a fire refused for a
//! payload value is `Validation`; every other failure is `Internal`, so it never reads as one of
//! the three.

use super::{
    DEFAULT_SIGNATURE_HEADER, IngestOutcome, PING_EVENT, WebhookIngressUseCases, verified_digest,
};
use crate::domain::clock;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::TriggerId;
use crate::domain::trigger::{Trigger, TriggerSource};
use async_trait::async_trait;
use scylla_extension::{
    Access, Authorized, Command, Committed, Describe, Draft, Persist, Prepare, Prepared, Run,
};
use std::collections::HashMap;
use tracing::warn;

/// No `Debug`: the headers carry the signature. `headers` is keyed by the lowercase name.
pub struct IngestWebhook {
    pub trigger_id: TriggerId,
    pub headers: HashMap<String, String>,
    pub delivery_id: Option<String>,
    pub event: Option<String>,
    pub body: Vec<u8>,
}

impl IngestWebhook {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
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

fn not_found(id: &TriggerId) -> DomainError {
    DomainError::not_found("Trigger", id)
}

fn internal(e: &DomainError) -> DomainError {
    DomainError::internal(e.to_string())
}

impl WebhookIngressUseCases {
    async fn enabled_trigger(&self, id: &TriggerId) -> DomainResult<Trigger> {
        match self.trigger_repo.find_by_id(id).await {
            Ok(t) if t.is_enabled() => Ok(t),
            Ok(_) => Err(not_found(id)),
            Err(e) if e.is_not_found() => Err(not_found(id)),
            Err(e) => Err(internal(&e)),
        }
    }
}

#[async_trait]
impl Run<Prepare<IngestWebhook>> for WebhookIngressUseCases {
    async fn run(&self, input: Authorized<IngestWebhook>) -> DomainResult<Prepared<IngestWebhook>> {
        let cmd = input.command();
        let trigger = self.enabled_trigger(&cmd.trigger_id).await?;
        let TriggerSource::Webhook(spec) = trigger.source() else {
            return Err(not_found(&cmd.trigger_id));
        };
        let header_name = spec.signature_header().unwrap_or(DEFAULT_SIGNATURE_HEADER);

        let secret = match self.trigger_repo.webhook_secret(&cmd.trigger_id).await {
            Ok(Some(enc)) => self.cipher.decrypt(&enc).map_err(|e| internal(&e))?,
            Ok(None) => {
                return Err(DomainError::internal(
                    "webhook trigger has no signing secret",
                ));
            }
            Err(e) => return Err(internal(&e)),
        };

        let bad_signature = || DomainError::unauthorized("invalid signature");
        let signature = cmd.header(header_name).ok_or_else(bad_signature)?;
        let digest = verified_digest(&secret, &cmd.body, signature).ok_or_else(bad_signature)?;
        // Verified before the ping check: a wrong secret fails on GitHub's creation-time ping, not on the first push.
        if cmd
            .event
            .as_deref()
            .is_some_and(|e| e.trim().eq_ignore_ascii_case(PING_EVENT))
        {
            return Ok(input.prepared(Draft::new(Delivery::Ping)));
        }

        let key = cmd
            .delivery_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map_or(digest, str::to_owned);
        let delivery = Delivery::Event {
            trigger_id: cmd.trigger_id.clone(),
            key,
            delivery_id: cmd.delivery_id.clone(),
            payload: serde_json::from_slice(&cmd.body).ok(),
        };
        Ok(input.prepared(Draft::new(delivery)))
    }
}

#[async_trait]
impl Run<Persist<IngestWebhook>> for WebhookIngressUseCases {
    async fn run(&self, input: Prepared<IngestWebhook>) -> DomainResult<Committed<IngestWebhook>> {
        input
            .commit(async |draft| {
                let Delivery::Event {
                    trigger_id,
                    key,
                    delivery_id,
                    payload,
                } = draft.into_inner()
                else {
                    return Ok(IngestOutcome::Ping);
                };
                let is_new = self
                    .delivery_repo
                    .record_or_detect(&trigger_id, &key, clock::now())
                    .await
                    .map_err(|e| internal(&e))?;
                if !is_new {
                    return Ok(IngestOutcome::Duplicate);
                }
                match self
                    .firing
                    .fire(&trigger_id, payload.as_ref(), delivery_id.as_deref())
                    .await
                {
                    Ok(job) => Ok(IngestOutcome::Fired(job.id().clone())),
                    Err(e) => {
                        if let Err(f) = self.delivery_repo.forget(&trigger_id, &key).await {
                            warn!(trigger_id = %trigger_id, error = %f, "could not forget a failed delivery");
                        }
                        match e {
                            DomainError::Validation(_) => Err(e),
                            e => match self.enabled_trigger(&trigger_id).await {
                                Err(gone) if gone.is_not_found() => Err(gone),
                                _ => Err(internal(&e)),
                            },
                        }
                    }
                }
            })
            .await
    }
}
