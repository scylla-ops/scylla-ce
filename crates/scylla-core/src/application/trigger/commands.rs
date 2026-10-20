//! The trigger's writes. One block per command, in the order it runs: the struct, its
//! access, its payload types, what `Prepare` builds, what `Persist` writes.

use super::TriggerUseCases;
use crate::application::actions::service_only;
use crate::domain::clock;
use crate::domain::errors::DomainResult;
use crate::domain::ids::{OrganizationId, PipelineId, TriggerId};
use crate::domain::permission::Permission;
use crate::domain::trigger::{FireObservation, Trigger, TriggerInput, TriggerName, TriggerSource};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scylla_extension::{
    Access, Authorized, Command, Committed, Deleted, Describe, Draft, Persist, Prepare, Prepared,
    Run,
};
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

#[async_trait]
impl Run<Prepare<CreateTrigger>> for TriggerUseCases {
    async fn run(&self, input: Authorized<CreateTrigger>) -> DomainResult<Prepared<CreateTrigger>> {
        let cmd = input.command();
        let pipeline = self.pipeline_repo.find_by_id(&cmd.pipeline_id).await?;
        let project = self.project_repo.find_by_id(pipeline.project_id()).await?;

        let mut trigger = Trigger::create(
            cmd.pipeline_id.clone(),
            cmd.name.clone(),
            cmd.source.clone(),
            cmd.inputs.clone(),
        )?;
        self.schedule_next(&mut trigger)?;

        let (webhook_secret, webhook_secret_enc) = match trigger.source() {
            TriggerSource::Webhook(_) => {
                let plaintext = generate_webhook_secret();
                let enc = self.cipher.encrypt(&plaintext)?;
                (Some(plaintext), Some(enc))
            }
            TriggerSource::Cron(_) => (None, None),
        };
        Ok(input.prepared(Draft::new(NewTrigger {
            trigger,
            organization_id: project.organization_id().clone(),
            webhook_secret,
            webhook_secret_enc,
        })))
    }
}

#[async_trait]
impl Run<Persist<CreateTrigger>> for TriggerUseCases {
    async fn run(&self, input: Prepared<CreateTrigger>) -> DomainResult<Committed<CreateTrigger>> {
        input
            .commit(async |draft| {
                let NewTrigger {
                    trigger,
                    organization_id,
                    webhook_secret,
                    webhook_secret_enc,
                } = draft.into_inner();
                self.ensure_runner_app(&organization_id).await?;
                let stored = self
                    .trigger_repo
                    .create(&trigger, webhook_secret_enc.as_deref())
                    .await?;
                Ok(CreatedTrigger {
                    trigger: stored,
                    webhook_secret,
                })
            })
            .await
    }
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

#[async_trait]
impl Run<Prepare<UpdateTrigger>> for TriggerUseCases {
    async fn run(&self, input: Authorized<UpdateTrigger>) -> DomainResult<Prepared<UpdateTrigger>> {
        let cmd = input.command();
        let mut trigger = self.trigger_repo.find_by_id(&cmd.id).await?;
        trigger.update(cmd.name.clone(), cmd.source.clone(), cmd.inputs.clone())?;
        // Re-anchor from now so a changed expression takes effect at once.
        self.schedule_next(&mut trigger)?;
        Ok(input.prepared(Draft::new(trigger)))
    }
}

#[async_trait]
impl Run<Persist<UpdateTrigger>> for TriggerUseCases {
    async fn run(&self, input: Prepared<UpdateTrigger>) -> DomainResult<Committed<UpdateTrigger>> {
        input
            .commit(async |draft| self.trigger_repo.update(&draft.into_inner()).await)
            .await
    }
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

#[async_trait]
impl Run<Prepare<SetTriggerEnabled>> for TriggerUseCases {
    async fn run(
        &self,
        input: Authorized<SetTriggerEnabled>,
    ) -> DomainResult<Prepared<SetTriggerEnabled>> {
        let cmd = input.command();
        let mut trigger = self.trigger_repo.find_by_id(&cmd.id).await?;
        // Re-anchor from now: no catch-up fire at a stale past time.
        if cmd.enabled {
            trigger.enable();
            self.schedule_next(&mut trigger)?;
        } else {
            trigger.disable();
        }
        Ok(input.prepared(Draft::new(trigger)))
    }
}

#[async_trait]
impl Run<Persist<SetTriggerEnabled>> for TriggerUseCases {
    async fn run(
        &self,
        input: Prepared<SetTriggerEnabled>,
    ) -> DomainResult<Committed<SetTriggerEnabled>> {
        input
            .commit(async |draft| self.trigger_repo.update(&draft.into_inner()).await)
            .await
    }
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

#[async_trait]
impl Run<Prepare<DeleteTrigger>> for TriggerUseCases {
    async fn run(&self, input: Authorized<DeleteTrigger>) -> DomainResult<Prepared<DeleteTrigger>> {
        let trigger = self.trigger_repo.find_by_id(&input.command().id).await?;
        Ok(input.prepared(trigger))
    }
}

#[async_trait]
impl Run<Persist<DeleteTrigger>> for TriggerUseCases {
    async fn run(&self, input: Prepared<DeleteTrigger>) -> DomainResult<Committed<DeleteTrigger>> {
        input
            .commit(async |trigger| {
                self.trigger_repo.delete(&trigger).await?;
                Ok(Deleted::new(trigger))
            })
            .await
    }
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

#[async_trait]
impl Run<Prepare<RecordTriggerFire>> for TriggerUseCases {
    async fn run(
        &self,
        input: Authorized<RecordTriggerFire>,
    ) -> DomainResult<Prepared<RecordTriggerFire>> {
        let observation = FireObservation {
            fired_at: clock::now(),
            status: input.command().status.to_owned(),
        };
        Ok(input.prepared(Draft::new(observation)))
    }
}

#[async_trait]
impl Run<Persist<RecordTriggerFire>> for TriggerUseCases {
    async fn run(
        &self,
        input: Prepared<RecordTriggerFire>,
    ) -> DomainResult<Committed<RecordTriggerFire>> {
        let id = input.command().id.clone();
        input
            .commit(async |draft| {
                let observation = draft.into_inner();
                self.trigger_repo.record_fire(&id, &observation).await?;
                Ok(observation)
            })
            .await
    }
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

#[async_trait]
impl Run<Prepare<ScheduleCronTriggers>> for TriggerUseCases {
    async fn run(
        &self,
        input: Authorized<ScheduleCronTriggers>,
    ) -> DomainResult<Prepared<ScheduleCronTriggers>> {
        service_only(input.caller())?;
        Ok(input.prepared(clock::now()))
    }
}

#[async_trait]
impl Run<Persist<ScheduleCronTriggers>> for TriggerUseCases {
    async fn run(
        &self,
        input: Prepared<ScheduleCronTriggers>,
    ) -> DomainResult<Committed<ScheduleCronTriggers>> {
        input
            .commit(async |now| {
                self.trigger_repo
                    .seed_cron(&|trigger| self.next_cron_fire(trigger, now))
                    .await
            })
            .await
    }
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

#[async_trait]
impl Run<Prepare<ClaimDueTriggers>> for TriggerUseCases {
    async fn run(
        &self,
        input: Authorized<ClaimDueTriggers>,
    ) -> DomainResult<Prepared<ClaimDueTriggers>> {
        service_only(input.caller())?;
        let staged = (input.command().now, input.command().limit);
        Ok(input.prepared(staged))
    }
}

#[async_trait]
impl Run<Persist<ClaimDueTriggers>> for TriggerUseCases {
    async fn run(
        &self,
        input: Prepared<ClaimDueTriggers>,
    ) -> DomainResult<Committed<ClaimDueTriggers>> {
        input
            .commit(async |(now, limit)| {
                self.trigger_repo
                    .claim_due_cron(now, limit, &|trigger| self.next_cron_fire(trigger, now))
                    .await
            })
            .await
    }
}

fn generate_webhook_secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
