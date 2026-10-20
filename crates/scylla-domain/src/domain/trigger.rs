mod cron_spec;
mod input;
mod name;
mod source;
mod webhook_spec;

pub use cron_spec::*;
pub use input::*;
pub use name::*;
pub use source::*;
pub use webhook_spec::*;

use crate::domain::clock;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{PipelineId, TriggerId};
use crate::domain::pipeline::MAX_ENV_VARS;
use chrono::{DateTime, Utc};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerActivation {
    Disabled,
    Enabled { next_fire_at: Option<DateTime<Utc>> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FireObservation {
    pub fired_at: DateTime<Utc>,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct Trigger {
    id: TriggerId,
    pipeline_id: PipelineId,
    name: TriggerName,
    source: TriggerSource,
    inputs: Vec<TriggerInput>,
    activation: TriggerActivation,
    last_observation: Option<FireObservation>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    /// The row version the value was read at. The store checks it on every edit and bumps
    /// it itself; the domain never changes it. A fire and a schedule advance are not edits.
    version: u64,
}

impl Trigger {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn from_persistence(
        id: TriggerId,
        pipeline_id: PipelineId,
        name: TriggerName,
        source: TriggerSource,
        inputs: Vec<TriggerInput>,
        enabled: bool,
        next_fire_at: Option<DateTime<Utc>>,
        last_fired_at: Option<DateTime<Utc>>,
        last_status: Option<String>,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        version: u64,
    ) -> Self {
        let activation = if enabled {
            TriggerActivation::Enabled { next_fire_at }
        } else {
            TriggerActivation::Disabled
        };
        let last_observation = last_fired_at.map(|fired_at| FireObservation {
            fired_at,
            status: last_status.unwrap_or_default(),
        });
        Self {
            id,
            pipeline_id,
            name,
            source,
            inputs,
            activation,
            last_observation,
            created_at,
            updated_at,
            version,
        }
    }

    pub fn create(
        pipeline_id: PipelineId,
        name: TriggerName,
        source: TriggerSource,
        inputs: Vec<TriggerInput>,
    ) -> DomainResult<Self> {
        Self::validate_inputs(&source, &inputs)?;

        let now = clock::now();
        Ok(Self {
            id: TriggerId::generate(),
            pipeline_id,
            name,
            source,
            inputs,
            activation: TriggerActivation::Enabled { next_fire_at: None },
            last_observation: None,
            created_at: now,
            updated_at: now,
            version: 0,
        })
    }

    pub fn update(
        &mut self,
        name: TriggerName,
        source: TriggerSource,
        inputs: Vec<TriggerInput>,
    ) -> DomainResult<()> {
        if source.kind() != self.source.kind() {
            return Err(DomainError::business_rule(
                "Trigger source kind is immutable; delete and recreate to change it",
            ));
        }
        Self::validate_inputs(&source, &inputs)?;
        self.name = name;
        self.source = source;
        self.inputs = inputs;
        self.updated_at = clock::now();
        Ok(())
    }

    /// Starts unscheduled; the use case computes the next occurrence.
    pub fn enable(&mut self) {
        self.activation = TriggerActivation::Enabled { next_fire_at: None };
        self.updated_at = clock::now();
    }

    pub fn disable(&mut self) {
        self.activation = TriggerActivation::Disabled;
        self.updated_at = clock::now();
    }

    pub fn set_next_fire_at(&mut self, next_fire_at: Option<DateTime<Utc>>) {
        if let TriggerActivation::Enabled { next_fire_at: slot } = &mut self.activation {
            *slot = next_fire_at;
            self.updated_at = clock::now();
        }
    }

    fn validate_inputs(source: &TriggerSource, inputs: &[TriggerInput]) -> DomainResult<()> {
        if inputs.len() > MAX_ENV_VARS {
            return Err(DomainError::validation(format!(
                "Trigger cannot have more than {MAX_ENV_VARS} inputs"
            )));
        }
        let mut seen = HashSet::new();
        for input in inputs {
            if !seen.insert(input.key()) {
                return Err(DomainError::validation(format!(
                    "Duplicate input key: {}",
                    input.key()
                )));
            }
            if matches!(source, TriggerSource::Cron(_))
                && matches!(input.source(), TriggerInputSource::JsonPointer(_))
            {
                return Err(DomainError::validation(format!(
                    "Input '{}' uses a JSON pointer, which requires a webhook source (cron has no payload)",
                    input.key()
                )));
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn id(&self) -> &TriggerId {
        &self.id
    }

    #[must_use]
    pub fn pipeline_id(&self) -> &PipelineId {
        &self.pipeline_id
    }

    #[must_use]
    pub fn name(&self) -> &TriggerName {
        &self.name
    }

    #[must_use]
    pub fn source(&self) -> &TriggerSource {
        &self.source
    }

    #[must_use]
    pub fn inputs(&self) -> &[TriggerInput] {
        &self.inputs
    }

    #[must_use]
    pub fn activation(&self) -> &TriggerActivation {
        &self.activation
    }

    #[must_use]
    pub fn last_observation(&self) -> Option<&FireObservation> {
        self.last_observation.as_ref()
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        matches!(self.activation, TriggerActivation::Enabled { .. })
    }

    #[must_use]
    pub fn next_fire_at(&self) -> Option<DateTime<Utc>> {
        match &self.activation {
            TriggerActivation::Enabled { next_fire_at } => *next_fire_at,
            TriggerActivation::Disabled => None,
        }
    }

    #[must_use]
    pub fn last_fired_at(&self) -> Option<DateTime<Utc>> {
        self.last_observation.as_ref().map(|o| o.fired_at)
    }

    #[must_use]
    pub fn last_status(&self) -> Option<&str> {
        self.last_observation.as_ref().map(|o| o.status.as_str())
    }

    #[must_use]
    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    #[must_use]
    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }

    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::pipeline::EnvKey;
    use crate::domain::trigger::{CronSpec, WebhookSpec};

    fn pipeline_id() -> PipelineId {
        PipelineId::generate()
    }

    fn name(n: &str) -> TriggerName {
        TriggerName::new(n).unwrap()
    }

    fn cron() -> TriggerSource {
        TriggerSource::Cron(CronSpec::new("0 9 * * *").unwrap())
    }

    fn webhook() -> TriggerSource {
        TriggerSource::Webhook(WebhookSpec::new(None).unwrap())
    }

    fn key(k: &str) -> EnvKey {
        EnvKey::new(k).unwrap()
    }

    #[test]
    fn create_cron_trigger_is_enabled_and_unscheduled() {
        let t = Trigger::create(pipeline_id(), name("nightly"), cron(), vec![]).unwrap();
        assert!(t.is_enabled());
        assert!(t.next_fire_at().is_none());
        assert!(t.last_fired_at().is_none());
    }

    #[test]
    fn cron_rejects_json_pointer_inputs() {
        let inputs = vec![TriggerInput::json_pointer(key("GIT_COMMIT"), "/after").unwrap()];
        let err = Trigger::create(pipeline_id(), name("bad"), cron(), inputs);
        assert!(err.is_err());
    }

    #[test]
    fn webhook_accepts_json_pointer_inputs() {
        let inputs = vec![TriggerInput::json_pointer(key("GIT_COMMIT"), "/after").unwrap()];
        assert!(Trigger::create(pipeline_id(), name("on-push"), webhook(), inputs).is_ok());
    }

    #[test]
    fn cron_accepts_literal_inputs() {
        let inputs = vec![TriggerInput::literal(key("RUN_MODE"), "nightly").unwrap()];
        assert!(Trigger::create(pipeline_id(), name("nightly"), cron(), inputs).is_ok());
    }

    #[test]
    fn rejects_duplicate_input_keys() {
        let inputs = vec![
            TriggerInput::literal(key("RUN_MODE"), "a").unwrap(),
            TriggerInput::literal(key("RUN_MODE"), "b").unwrap(),
        ];
        assert!(Trigger::create(pipeline_id(), name("dup"), cron(), inputs).is_err());
    }

    #[test]
    fn rejects_too_many_inputs() {
        let inputs = |n: usize| {
            (0..n)
                .map(|i| TriggerInput::literal(key(&format!("K{i}")), "v").unwrap())
                .collect()
        };
        assert!(Trigger::create(pipeline_id(), name("ok"), cron(), inputs(MAX_ENV_VARS)).is_ok());
        assert!(
            Trigger::create(pipeline_id(), name("big"), cron(), inputs(MAX_ENV_VARS + 1)).is_err()
        );
    }

    #[test]
    fn update_cannot_change_kind() {
        let mut t = Trigger::create(pipeline_id(), name("nightly"), cron(), vec![]).unwrap();
        let err = t.update(name("nightly"), webhook(), vec![]);
        assert!(err.is_err());
        assert_eq!(t.source().kind().as_str(), "cron");
    }

    #[test]
    fn update_same_kind_succeeds() {
        let mut t = Trigger::create(pipeline_id(), name("nightly"), cron(), vec![]).unwrap();
        let new_source = TriggerSource::Cron(CronSpec::new("0 0 * * *").unwrap());
        t.update(name("midnight"), new_source, vec![]).unwrap();
        assert_eq!(t.name().as_str(), "midnight");
    }

    #[test]
    fn disable_drops_the_schedule() {
        let mut t = Trigger::create(pipeline_id(), name("nightly"), cron(), vec![]).unwrap();
        t.set_next_fire_at(Some(clock::now()));
        assert!(t.next_fire_at().is_some());

        t.disable();
        assert!(!t.is_enabled());
        assert!(t.next_fire_at().is_none());

        t.set_next_fire_at(Some(clock::now()));
        assert!(t.next_fire_at().is_none());
    }
}
