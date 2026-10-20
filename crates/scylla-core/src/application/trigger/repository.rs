use crate::domain::errors::DomainResult;
use crate::domain::ids::{PipelineId, TriggerId};
use crate::domain::trigger::{FireObservation, Trigger};
use async_trait::async_trait;
use chrono::{DateTime, Utc};

pub type NextFire<'a> = dyn Fn(&Trigger) -> DomainResult<DateTime<Utc>> + Sync + 'a;

#[async_trait]
pub trait TriggerRepository: Send + Sync {
    async fn create(
        &self,
        trigger: &Trigger,
        webhook_secret_enc: Option<&[u8]>,
    ) -> DomainResult<Trigger>;

    async fn find_by_id(&self, id: &TriggerId) -> DomainResult<Trigger>;

    async fn webhook_secret(&self, id: &TriggerId) -> DomainResult<Option<Vec<u8>>>;

    /// Writes the edit (name, source, inputs, activation) only if the row still carries
    /// `trigger.version()`, and returns the row with the bumped version. Never writes the last
    /// fire. A stale value is `Stale`; a missing row is `NotFound`.
    async fn update(&self, trigger: &Trigger) -> DomainResult<Trigger>;

    /// Same version rule as `update`.
    async fn delete(&self, trigger: &Trigger) -> DomainResult<()>;

    async fn list_by_pipeline(&self, pipeline_id: &PipelineId) -> DomainResult<Vec<Trigger>>;

    /// Writes only the last fire, whatever the version. A deleted trigger records nothing.
    async fn record_fire(&self, id: &TriggerId, observation: &FireObservation) -> DomainResult<()>;

    /// Gives each enabled cron trigger without a next fire time its first one. Same locking as
    /// `claim_due_cron`; returns the rows as read.
    async fn seed_cron(&self, compute_next: &NextFire<'_>) -> DomainResult<Vec<Trigger>>;

    /// `FOR UPDATE SKIP LOCKED` and the advance in the same transaction: no pass or instance
    /// re-claims a row. The advance writes only the next fire time; returns the rows as read.
    async fn claim_due_cron(
        &self,
        now: DateTime<Utc>,
        limit: i64,
        compute_next: &NextFire<'_>,
    ) -> DomainResult<Vec<Trigger>>;
}
