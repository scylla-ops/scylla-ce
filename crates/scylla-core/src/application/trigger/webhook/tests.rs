//! The webhook ingress through the engine, on stub ports.

use super::*;
use crate::application::SecretCipher;
use crate::application::trigger::NextFire;
use crate::domain::caller::CallerContext;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{PipelineId, TriggerId};
use crate::domain::job::Job;
use crate::domain::trigger::{FireObservation, Trigger, TriggerName, TriggerSource, WebhookSpec};
use crate::test_support::authz::{DenyingPermissionService, actions};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Mutex;

const KEY: &str = "key";
const MSG: &[u8] = b"The quick brown fox jumps over the lazy dog";
const SIG: &str = "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8";

/// `trigger` is what the next read returns, so a test can change it between two reads.
struct StubRepo {
    trigger: Mutex<Trigger>,
}

#[async_trait]
impl TriggerRepository for StubRepo {
    async fn find_by_id(&self, id: &TriggerId) -> DomainResult<Trigger> {
        let trigger = self.trigger.lock().unwrap().clone();
        if id == trigger.id() {
            Ok(trigger)
        } else {
            Err(DomainError::not_found("Trigger", id))
        }
    }
    async fn webhook_secret(&self, _: &TriggerId) -> DomainResult<Option<Vec<u8>>> {
        Ok(Some(KEY.as_bytes().to_vec()))
    }
    async fn create(&self, _: &Trigger, _: Option<&[u8]>) -> DomainResult<Trigger> {
        unreachable!("no trigger create in a webhook ingest")
    }
    async fn update(&self, _: &Trigger) -> DomainResult<Trigger> {
        unreachable!("no trigger update in a webhook ingest")
    }
    async fn delete(&self, _: &Trigger) -> DomainResult<()> {
        unreachable!("no trigger delete in a webhook ingest")
    }
    async fn list_by_pipeline(&self, _: &PipelineId) -> DomainResult<Vec<Trigger>> {
        unreachable!("no trigger listing in a webhook ingest")
    }
    async fn record_fire(&self, _: &TriggerId, _: &FireObservation) -> DomainResult<()> {
        unreachable!("the fire records its own outcome")
    }
    async fn seed_cron(&self, _: &NextFire<'_>) -> DomainResult<Vec<Trigger>> {
        unreachable!("no cron scheduling in a webhook ingest")
    }
    async fn claim_due_cron(
        &self,
        _: DateTime<Utc>,
        _: i64,
        _: &NextFire<'_>,
    ) -> DomainResult<Vec<Trigger>> {
        unreachable!("no cron claim in a webhook ingest")
    }
}

struct StubDeliveries {
    recorded: Mutex<Vec<String>>,
}

#[async_trait]
impl TriggerDeliveryRepository for StubDeliveries {
    async fn record_or_detect(
        &self,
        _: &TriggerId,
        delivery_id: &str,
        _: DateTime<Utc>,
    ) -> DomainResult<bool> {
        let mut seen = self.recorded.lock().unwrap();
        let is_new = !seen.iter().any(|d| d == delivery_id);
        if is_new {
            seen.push(delivery_id.to_string());
        }
        Ok(is_new)
    }
    async fn forget(&self, _: &TriggerId, delivery_id: &str) -> DomainResult<()> {
        self.recorded.lock().unwrap().retain(|d| d != delivery_id);
        Ok(())
    }
}

struct PlainCipher;

impl SecretCipher for PlainCipher {
    fn encrypt(&self, plaintext: &str) -> DomainResult<Vec<u8>> {
        Ok(plaintext.as_bytes().to_vec())
    }
    fn decrypt(&self, ciphertext: &[u8]) -> DomainResult<String> {
        Ok(String::from_utf8(ciphertext.to_vec()).unwrap())
    }
}

/// Counts the fires; the first one fails with `failure` when it is set, and disables the
/// trigger of `disables` first.
struct StubFiring {
    job: Job,
    fired: Mutex<u32>,
    failure: Mutex<Option<DomainError>>,
    disables: Mutex<Option<Arc<StubRepo>>>,
}

#[async_trait]
impl TriggerFiring for StubFiring {
    async fn fire(
        &self,
        _: &TriggerId,
        _: Option<&serde_json::Value>,
        _: Option<&str>,
    ) -> DomainResult<Job> {
        *self.fired.lock().unwrap() += 1;
        if let Some(repo) = self.disables.lock().unwrap().take() {
            repo.trigger.lock().unwrap().disable();
        }
        match self.failure.lock().unwrap().take() {
            Some(e) => Err(e),
            None => Ok(self.job.clone()),
        }
    }
}

struct Lab {
    ingress: WebhookIngressUseCases,
    repo: Arc<StubRepo>,
    deliveries: Arc<StubDeliveries>,
    firing: Arc<StubFiring>,
    trigger_id: TriggerId,
}

impl Lab {
    async fn ingest(
        &self,
        signature: Option<&str>,
        delivery_id: Option<&str>,
        event: Option<&str>,
    ) -> DomainResult<IngestOutcome> {
        self.ingest_to(&self.trigger_id, signature, delivery_id, event)
            .await
    }

    async fn ingest_to(
        &self,
        trigger_id: &TriggerId,
        signature: Option<&str>,
        delivery_id: Option<&str>,
        event: Option<&str>,
    ) -> DomainResult<IngestOutcome> {
        let headers = signature
            .map(|s| {
                let name = DEFAULT_SIGNATURE_HEADER.to_ascii_lowercase();
                HashMap::from([(name, s.to_string())])
            })
            .unwrap_or_default();
        let command = IngestWebhook {
            trigger_id: trigger_id.clone(),
            headers,
            delivery_id: delivery_id.map(str::to_string),
            event: event.map(str::to_string),
            body: MSG.to_vec(),
        };
        actions(Arc::new(DenyingPermissionService::new()))
            .run(&self.ingress, &CallerContext::Anonymous, command)
            .await
    }
}

fn lab() -> Lab {
    use crate::test_support::{
        jobs::job, organizations::org, pipelines::pipeline, projects::project,
    };
    let trigger = Trigger::create(
        PipelineId::new("p"),
        TriggerName::new("hook").unwrap(),
        TriggerSource::Webhook(WebhookSpec::new(None).unwrap()),
        vec![],
    )
    .unwrap();
    let trigger_id = trigger.id().clone();
    let deliveries = Arc::new(StubDeliveries {
        recorded: Mutex::new(vec![]),
    });
    let firing = Arc::new(StubFiring {
        job: job(&pipeline(&project(&org("o"), "p"))),
        fired: Mutex::new(0),
        failure: Mutex::new(None),
        disables: Mutex::new(None),
    });
    let repo = Arc::new(StubRepo {
        trigger: Mutex::new(trigger),
    });
    let ingress = WebhookIngressUseCases::new(
        repo.clone(),
        deliveries.clone(),
        Arc::new(PlainCipher),
        firing.clone(),
    );
    Lab {
        ingress,
        repo,
        deliveries,
        firing,
        trigger_id,
    }
}

#[tokio::test]
async fn signed_ping_answers_without_firing_or_recording() {
    let h = lab();
    let outcome = h
        .ingest(Some(SIG), Some("d-1"), Some("ping"))
        .await
        .unwrap();
    assert!(matches!(outcome, IngestOutcome::Ping));
    assert_eq!(*h.firing.fired.lock().unwrap(), 0);
    assert!(h.deliveries.recorded.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ping_event_name_is_trimmed_and_case_insensitive() {
    let h = lab();
    let outcome = h.ingest(Some(SIG), None, Some(" Ping ")).await.unwrap();
    assert!(matches!(outcome, IngestOutcome::Ping));
    assert_eq!(*h.firing.fired.lock().unwrap(), 0);
}

#[tokio::test]
async fn mis_signed_ping_is_rejected_before_the_event_check() {
    let h = lab();
    let err = h
        .ingest(Some("sha256=00"), None, Some("ping"))
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Unauthorized(_)));
    assert_eq!(*h.firing.fired.lock().unwrap(), 0);
}

#[tokio::test]
async fn unsigned_ping_is_rejected_before_the_event_check() {
    let h = lab();
    let err = h.ingest(None, None, Some("ping")).await.unwrap_err();
    assert!(matches!(err, DomainError::Unauthorized(_)));
    assert_eq!(*h.firing.fired.lock().unwrap(), 0);
}

#[tokio::test]
async fn other_events_and_no_event_still_fire() {
    let h = lab();
    let push = h
        .ingest(Some(SIG), Some("d-1"), Some("push"))
        .await
        .unwrap();
    assert!(matches!(push, IngestOutcome::Fired(_)));
    let bare = h.ingest(Some(SIG), Some("d-2"), None).await.unwrap();
    assert!(matches!(bare, IngestOutcome::Fired(_)));
    assert_eq!(*h.firing.fired.lock().unwrap(), 2);
}

#[tokio::test]
async fn a_repeated_delivery_is_a_duplicate_and_fires_once() {
    let h = lab();
    h.ingest(Some(SIG), Some("d-1"), None).await.unwrap();
    let again = h.ingest(Some(SIG), Some(" d-1 "), None).await.unwrap();
    assert!(matches!(again, IngestOutcome::Duplicate));
    assert_eq!(*h.firing.fired.lock().unwrap(), 1);
}

#[tokio::test]
async fn an_unknown_trigger_is_not_found_without_asking_a_permission() {
    let h = lab();
    let err = h
        .ingest_to(&TriggerId::new("missing"), Some(SIG), None, None)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotFound(_)));
    assert!(h.deliveries.recorded.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_failed_fire_forgets_the_delivery_so_a_retry_fires() {
    let h = lab();
    *h.firing.failure.lock().unwrap() = Some(DomainError::infrastructure("db down"));

    let err = h.ingest(Some(SIG), Some("d-1"), None).await.unwrap_err();
    assert!(matches!(err, DomainError::Internal(_)));
    assert!(h.deliveries.recorded.lock().unwrap().is_empty());

    let retry = h.ingest(Some(SIG), Some("d-1"), None).await.unwrap();
    assert!(matches!(retry, IngestOutcome::Fired(_)));
    assert_eq!(*h.firing.fired.lock().unwrap(), 2);
}

#[tokio::test]
async fn a_trigger_disabled_while_the_delivery_is_in_flight_is_not_found() {
    let h = lab();
    *h.firing.disables.lock().unwrap() = Some(h.repo.clone());
    *h.firing.failure.lock().unwrap() = Some(DomainError::business_rule("trigger is disabled"));

    let err = h.ingest(Some(SIG), Some("d-1"), None).await.unwrap_err();

    assert!(matches!(err, DomainError::NotFound(_)), "{err:?}");
    assert!(h.deliveries.recorded.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_fire_refused_for_a_payload_value_stays_a_validation_error() {
    let h = lab();
    *h.firing.failure.lock().unwrap() = Some(DomainError::validation("NUL in an input"));

    let err = h.ingest(Some(SIG), Some("d-1"), None).await.unwrap_err();

    assert!(matches!(err, DomainError::Validation(_)));
    assert!(h.deliveries.recorded.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_prefixed_and_bare_signature_are_one_delivery() {
    let h = lab();
    let first = h.ingest(Some(SIG), None, None).await.unwrap();
    let prefixed = format!("SHA256={}", SIG.to_uppercase());
    let again = h.ingest(Some(&prefixed), None, None).await.unwrap();

    assert!(matches!(first, IngestOutcome::Fired(_)));
    assert!(matches!(again, IngestOutcome::Duplicate));
    assert_eq!(*h.firing.fired.lock().unwrap(), 1);
    assert_eq!(
        *h.deliveries.recorded.lock().unwrap(),
        vec![SIG.to_string()]
    );
}

#[test]
fn accepts_correct_signature_with_and_without_prefix() {
    let digest = Some(SIG.to_string());
    assert_eq!(verified_digest(KEY, MSG, SIG), digest);
    assert_eq!(verified_digest(KEY, MSG, &format!("sha256={SIG}")), digest);
    assert_eq!(verified_digest(KEY, MSG, &format!("sha256= {SIG}")), digest);
}

#[test]
fn uppercase_hex_and_prefix_verify() {
    let upper = SIG.to_uppercase();
    let digest = Some(SIG.to_string());
    assert_eq!(verified_digest(KEY, MSG, &upper), digest);
    assert_eq!(
        verified_digest(KEY, MSG, &format!("SHA256={upper}")),
        digest
    );
    assert_eq!(verified_digest(KEY, MSG, &format!("Sha256={SIG}")), digest);
}

#[test]
fn rejects_tampered_body_wrong_key_and_garbage() {
    assert_eq!(verified_digest(KEY, b"tampered", SIG), None);
    assert_eq!(verified_digest("wrong-key", MSG, SIG), None);
    assert_eq!(verified_digest(KEY, MSG, "not-hex"), None);
    assert_eq!(verified_digest(KEY, MSG, &SIG[..62]), None);
    assert_eq!(verified_digest(KEY, MSG, "sha256="), None);
    assert_eq!(verified_digest(KEY, MSG, ""), None);
}
