//! The trigger's writes. Each command, its access and its payload types. The strata module of the
//! slice runs it.

use crate::domain::ids::{OrganizationId, PipelineId, TriggerId};
use crate::domain::permission::Permission;
use crate::domain::trigger::{FireObservation, Trigger, TriggerInput, TriggerName, TriggerSource};
use chrono::{DateTime, Utc};
use scylla_extension::{Access, Command, Deleted, Describe, Draft};
use uuid::Uuid;

#[derive(Debug)]
pub struct CreateTrigger {
    pub pipeline_id: PipelineId,
    pub name: TriggerName,
    pub source: TriggerSource,
    pub inputs: Vec<TriggerInput>,
}

/// No `Debug`: `webhook_secret` is the plaintext the caller sees once. The runner app of the
/// organization is provisioned with the trigger, so its organization is staged too.
pub struct NewTrigger {
    pub trigger: Trigger,
    pub organization_id: OrganizationId,
    pub webhook_secret: Option<String>,
    pub webhook_secret_enc: Option<Vec<u8>>,
}

/// No `Debug`: `webhook_secret` is the plaintext the caller sees once.
pub struct CreatedTrigger {
    pub trigger: Trigger,
    pub webhook_secret: Option<String>,
}

// Managing triggers must not give run rights, so a create and an update also ask for them.
impl Describe for CreateTrigger {
    fn access(&self) -> Access {
        Access::RequiresAll(vec![
            Permission::ManageTriggers(self.pipeline_id.clone()),
            Permission::RunPipeline(self.pipeline_id.clone()),
        ])
    }
}

impl Command for CreateTrigger {
    type Staged = Draft<NewTrigger>;
    type Committed = CreatedTrigger;
}

#[derive(Debug)]
pub struct UpdateTrigger {
    pub id: TriggerId,
    pub name: TriggerName,
    pub source: TriggerSource,
    pub inputs: Vec<TriggerInput>,
}

impl Describe for UpdateTrigger {
    fn access(&self) -> Access {
        Access::RequiresAll(vec![
            Permission::ManageTrigger(self.id.clone()),
            Permission::RunTriggerPipeline(self.id.clone()),
        ])
    }
}

impl Command for UpdateTrigger {
    type Staged = Draft<Trigger>;
    type Committed = Trigger;
}

#[derive(Debug)]
pub struct SetTriggerEnabled {
    pub id: TriggerId,
    pub enabled: bool,
}

impl Describe for SetTriggerEnabled {
    fn access(&self) -> Access {
        Access::Requires(Permission::ManageTrigger(self.id.clone()))
    }
}

impl Command for SetTriggerEnabled {
    type Staged = Draft<Trigger>;
    type Committed = Trigger;
}

#[derive(Debug)]
pub struct DeleteTrigger {
    pub id: TriggerId,
}

impl Describe for DeleteTrigger {
    fn access(&self) -> Access {
        Access::Requires(Permission::ManageTrigger(self.id.clone()))
    }
}

impl Command for DeleteTrigger {
    type Staged = Trigger;
    type Committed = Deleted<Trigger>;
}

/// The outcome of a fire, sent by the trigger firer as a service. Only the observation is
/// written: an edit that committed during the run stays.
#[derive(Debug)]
pub struct RecordTriggerFire {
    pub id: TriggerId,
    pub status: &'static str,
}

impl Describe for RecordTriggerFire {
    fn access(&self) -> Access {
        Access::Requires(Permission::ManageTrigger(self.id.clone()))
    }
}

impl Command for RecordTriggerFire {
    type Staged = Draft<FireObservation>;
    type Committed = FireObservation;
}

/// One pass over the cron triggers that have no next fire time. A trigger whose next fire time
/// cannot be computed is logged and stays unscheduled.
#[derive(Debug)]
pub struct ScheduleCronTriggers;

impl Describe for ScheduleCronTriggers {
    fn access(&self) -> Access {
        Access::Authenticated
    }
}

impl Command for ScheduleCronTriggers {
    type Staged = DateTime<Utc>;
    type Committed = Vec<Trigger>;
}

/// One claim of the cron triggers due at `now`: the store advances each claimed row to its
/// next fire time in the same transaction, so no pass and no instance claims a row twice.
#[derive(Debug)]
pub struct ClaimDueTriggers {
    pub now: DateTime<Utc>,
    pub limit: i64,
}

impl Describe for ClaimDueTriggers {
    fn access(&self) -> Access {
        Access::Authenticated
    }
}

impl Command for ClaimDueTriggers {
    type Staged = (DateTime<Utc>, i64);
    type Committed = Vec<Trigger>;
}

pub fn generate_webhook_secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
