use crate::access::{CallerOf, Permissions};
use scylla_core::application::guards::service_only;
use scylla_core::application::trigger::fire::resolve_inputs;
use scylla_core::application::trigger::webhook::PING_EVENT;
use scylla_core::application::trigger::{
    CreatedTrigger, CronSchedule, DEFAULT_SIGNATURE_HEADER, IngestOutcome,
    TriggerDeliveryRepository, TriggerRepository, TriggerRun, TriggerUseCases,
    generate_webhook_secret, verified_digest,
};
use scylla_core::application::{
    AppRepository, PipelineRepository, ProjectRepository, SecretCipher,
};
use scylla_core::rest::webhook::WebhookDelivery;
use scylla_domain::domain::caller::{CallerContext, ServiceIdentity};
use scylla_domain::domain::clock;
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{PipelineId, TriggerId};
use scylla_domain::domain::job::{Job, JobOrigin};
use scylla_domain::domain::permission::Permission;
use scylla_domain::domain::trigger::{
    FireObservation, Trigger, TriggerInput, TriggerName, TriggerSource,
};
use scylla_extension::Access;
use sqlx::types::chrono::{DateTime, Utc};
use std::sync::Arc;
use strata::{BuildCx, BuildError, Component, Resource};

#[derive(Clone, Default)]
pub struct TriggerWebhookBase(pub Option<String>);

impl Resource for TriggerWebhookBase {
    const SECTION: &'static str = "webhook";
    async fn build(_: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Err(BuildError::msg(
            "the webhook base URL is given by the binary: give it with `Builder::provide`",
        ))
    }
}

pub struct TriggerWebhookLinks(Option<String>);

impl TriggerWebhookLinks {
    #[must_use]
    pub fn base(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

impl Component for TriggerWebhookLinks {
    async fn build(cx: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Ok(TriggerWebhookLinks(
            cx.resource::<TriggerWebhookBase>().await?.0,
        ))
    }
}

/// The action name of the run of a fire, as the `FireAudit` slot sees it.
pub const RUN_PIPELINE_WITH_INPUTS: &str = "run_pipeline_with_inputs";

/// The action name of the record of a fire, as the `FireAudit` slot sees it.
pub const RECORD_TRIGGER_FIRE: &str = "record_trigger_fire";

/// The audit of the inner runs of a fire. A fire runs the pipeline and records its outcome as
/// module methods, so no hook sees them. The fire gives each run to this slot after the run:
/// the action name, the caller, the access and the outcome.
#[strata::port]
pub trait FireAudit: Send + Sync + 'static {
    async fn record(
        &self,
        action: &'static str,
        caller: &CallerContext,
        access: &scylla_extension::Access,
        outcome: Result<(), &DomainError>,
    );
}

/// The CE audit of a fire: it keeps nothing. An edition overrides `Triggers::Audit`.
#[derive(Default)]
pub struct NoFireAudit;

impl FireAudit for NoFireAudit {
    async fn record(
        &self,
        _: &'static str,
        _: &CallerContext,
        _: &scylla_extension::Access,
        _: Result<(), &DomainError>,
    ) {
    }
}

#[strata::module(section = "triggers")]
pub trait Triggers: crate::Pipelines {
    #[component]
    type TriggerRepo = Arc<dyn TriggerRepository>;

    #[component]
    type TriggerPipelines = Arc<dyn PipelineRepository>;

    #[component]
    type TriggerProjects = Arc<dyn ProjectRepository>;

    #[component]
    type TriggerApps = Arc<dyn AppRepository>;

    #[component]
    type TriggerCipher = Arc<dyn SecretCipher>;

    #[component]
    type TriggerSchedule = Arc<dyn CronSchedule>;

    #[component]
    type TriggerLinks = TriggerWebhookLinks;

    #[component]
    type TriggerDeliveries = Arc<dyn TriggerDeliveryRepository>;

    #[component]
    type Audit: FireAudit = NoFireAudit;

    fn trigger_rules(&self) -> TriggerUseCases {
        TriggerUseCases::new(
            self.trigger_pipelines().clone(),
            self.trigger_projects().clone(),
            self.trigger_apps().clone(),
            self.trigger_schedule().clone(),
        )
    }

    async fn require_as(
        &self,
        caller: &CallerContext,
        access: scylla_extension::Access,
    ) -> DomainResult<()> {
        if access == scylla_extension::Access::Authenticated && *caller == CallerContext::Anonymous
        {
            return Err(DomainError::forbidden("Anonymous caller is not permitted"));
        }
        for permission in access.permissions() {
            self.permissions().check(caller, permission.clone()).await?;
        }
        Ok(())
    }

    async fn resolve_run(&self, id: &TriggerId) -> DomainResult<TriggerRun> {
        let trigger = self.trigger_repo().find_by_id(id).await?;
        if !trigger.is_enabled() {
            return Err(DomainError::business_rule("trigger is disabled"));
        }
        let runner = self.trigger_rules().trigger_runner(&trigger).await;
        Ok(TriggerRun { trigger, runner })
    }

    async fn run_as_runner(
        &self,
        trigger: &Trigger,
        runner: CallerContext,
        payload: Option<&serde_json::Value>,
        delivery_id: Option<&str>,
    ) -> DomainResult<Job> {
        let origin = match trigger.source() {
            TriggerSource::Cron(_) => JobOrigin::Cron {
                trigger_id: trigger.id().clone(),
            },
            TriggerSource::Webhook(_) => JobOrigin::Webhook {
                trigger_id: trigger.id().clone(),
                delivery_id: delivery_id.map(str::to_owned),
            },
        };
        let id = trigger.pipeline_id();
        let inputs = resolve_inputs(trigger, payload)?;
        let access = crate::pipelines::run_access(id);
        let outcome = async {
            self.require_as(&runner, access.clone()).await?;
            let job = self.new_job(id, origin, inputs).await?;
            self.start(&job).await
        }
        .await;
        self.audit()
            .record(
                RUN_PIPELINE_WITH_INPUTS,
                &runner,
                &access,
                outcome.as_ref().err().map_or(Ok(()), Err),
            )
            .await;
        outcome
    }

    async fn record_as(
        &self,
        caller: &CallerContext,
        id: &TriggerId,
        status: &'static str,
    ) -> DomainResult<()> {
        let access = manage_access(id);
        let outcome = async {
            self.require_as(caller, access.clone()).await?;
            let observation = FireObservation {
                fired_at: clock::now(),
                status: status.to_owned(),
            };
            self.trigger_repo().record_fire(id, &observation).await
        }
        .await;
        self.audit()
            .record(
                RECORD_TRIGGER_FIRE,
                caller,
                &access,
                outcome.as_ref().err().map_or(Ok(()), Err),
            )
            .await;
        outcome
    }

    async fn fire(&self, id: &TriggerId) -> DomainResult<Job> {
        self.fire_with(id, None, None).await
    }

    /// Fires the trigger `id` as the trigger firer service. `payload` gives the values of the
    /// JSON pointer inputs, and `delivery_id` goes in the webhook origin of the job.
    async fn fire_with(
        &self,
        id: &TriggerId,
        payload: Option<&serde_json::Value>,
        delivery_id: Option<&str>,
    ) -> DomainResult<Job> {
        let service = CallerContext::Service(ServiceIdentity::trigger_firer());
        let TriggerRun { trigger, runner } = self.resolve_run(id).await?;
        let outcome = match runner {
            Ok(runner) => {
                self.run_as_runner(&trigger, CallerContext::App(runner), payload, delivery_id)
                    .await
            }
            Err(e) => Err(e),
        };
        let status = if outcome.is_ok() { "ok" } else { "error" };
        if let Err(e) = self.record_as(&service, id, status).await {
            tracing::warn!(trigger_id = %id, error = %e, "failed to record trigger fire status");
        }
        outcome
    }

    async fn enabled_trigger(&self, id: &TriggerId) -> DomainResult<Trigger> {
        match self.trigger_repo().find_by_id(id).await {
            Ok(t) if t.is_enabled() => Ok(t),
            Ok(_) => Err(trigger_not_found(id)),
            Err(e) if e.is_not_found() => Err(trigger_not_found(id)),
            Err(e) => Err(internal(&e)),
        }
    }

    /// Verifies the signature of `delivery` before any write. A verified ping is
    /// `VerifiedDelivery::Ping`.
    async fn verify_delivery(&self, delivery: &WebhookDelivery) -> DomainResult<VerifiedDelivery> {
        let trigger = self.enabled_trigger(&delivery.trigger_id).await?;
        let TriggerSource::Webhook(spec) = trigger.source() else {
            return Err(trigger_not_found(&delivery.trigger_id));
        };
        let header_name = spec.signature_header().unwrap_or(DEFAULT_SIGNATURE_HEADER);
        let secret = match self
            .trigger_repo()
            .webhook_secret(&delivery.trigger_id)
            .await
        {
            Ok(Some(enc)) => self
                .trigger_cipher()
                .decrypt(&enc)
                .map_err(|e| internal(&e))?,
            Ok(None) => {
                return Err(DomainError::internal(
                    "webhook trigger has no signing secret",
                ));
            }
            Err(e) => return Err(internal(&e)),
        };
        let bad_signature = || DomainError::unauthorized("invalid signature");
        let signature = delivery
            .headers
            .get(&header_name.to_ascii_lowercase())
            .ok_or_else(bad_signature)?;
        let digest =
            verified_digest(&secret, &delivery.body, signature).ok_or_else(bad_signature)?;
        if delivery
            .event
            .as_deref()
            .is_some_and(|e| e.trim().eq_ignore_ascii_case(PING_EVENT))
        {
            return Ok(VerifiedDelivery::Ping);
        }
        let key = delivery
            .delivery_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map_or(digest, str::to_owned);
        Ok(VerifiedDelivery::Event {
            trigger_id: delivery.trigger_id.clone(),
            key,
            delivery_id: delivery.delivery_id.clone(),
            payload: serde_json::from_slice(&delivery.body).ok(),
        })
    }

    /// The webhook ingress: verify the signature, record the delivery under its dedupe key, then
    /// fire. A failed fire forgets the delivery, thus a retry fires. It is not a use case.
    async fn ingest_webhook(&self, delivery: WebhookDelivery) -> DomainResult<IngestOutcome> {
        let VerifiedDelivery::Event {
            trigger_id,
            key,
            delivery_id,
            payload,
        } = self.verify_delivery(&delivery).await?
        else {
            return Ok(IngestOutcome::Ping);
        };
        let is_new = self
            .trigger_deliveries()
            .record_or_detect(&trigger_id, &key, clock::now())
            .await
            .map_err(|e| internal(&e))?;
        if !is_new {
            return Ok(IngestOutcome::Duplicate);
        }
        match self
            .fire_with(&trigger_id, payload.as_ref(), delivery_id.as_deref())
            .await
        {
            Ok(job) => Ok(IngestOutcome::Fired(job.id().clone())),
            Err(e) => {
                if let Err(f) = self.trigger_deliveries().forget(&trigger_id, &key).await {
                    tracing::warn!(trigger_id = %trigger_id, error = %f, "could not forget a failed delivery");
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
    }

    #[usecase(write, access = create_trigger_access(&pipeline_id))]
    async fn create_trigger(
        &self,
        pipeline_id: PipelineId,
        name: TriggerName,
        source: TriggerSource,
        inputs: Vec<TriggerInput>,
    ) -> DomainResult<CreatedTrigger> {
        let rules = self.trigger_rules();
        let pipeline = self.trigger_pipelines().find_by_id(&pipeline_id).await?;
        let project = self
            .trigger_projects()
            .find_by_id(pipeline.project_id())
            .await?;
        let mut trigger = Trigger::create(pipeline_id, name, source, inputs)?;
        rules.schedule_next(&mut trigger)?;
        let (webhook_secret, webhook_secret_enc) = match trigger.source() {
            TriggerSource::Webhook(_) => {
                let plaintext = generate_webhook_secret();
                let enc = self.trigger_cipher().encrypt(&plaintext)?;
                (Some(plaintext), Some(enc))
            }
            TriggerSource::Cron(_) => (None, None),
        };
        rules.ensure_runner_app(project.organization_id()).await?;
        let stored = self
            .trigger_repo()
            .create(&trigger, webhook_secret_enc.as_deref())
            .await?;
        Ok(CreatedTrigger {
            trigger: stored,
            webhook_secret,
        })
    }

    #[usecase(read, access = manage_access(&id))]
    async fn get_trigger(&self, id: TriggerId) -> DomainResult<Trigger> {
        self.trigger_repo().find_by_id(&id).await
    }

    #[usecase(write, access = update_trigger_access(&id))]
    async fn update_trigger(
        &self,
        id: TriggerId,
        name: TriggerName,
        source: TriggerSource,
        inputs: Vec<TriggerInput>,
    ) -> DomainResult<Trigger> {
        let mut trigger = self.trigger_repo().find_by_id(&id).await?;
        trigger.update(name, source, inputs)?;
        self.trigger_rules().schedule_next(&mut trigger)?;
        self.trigger_repo().update(&trigger).await
    }

    #[usecase(write, access = manage_access(&id))]
    async fn set_trigger_enabled(&self, id: TriggerId, enabled: bool) -> DomainResult<Trigger> {
        let mut trigger = self.trigger_repo().find_by_id(&id).await?;
        if enabled {
            trigger.enable();
            self.trigger_rules().schedule_next(&mut trigger)?;
        } else {
            trigger.disable();
        }
        self.trigger_repo().update(&trigger).await
    }

    #[usecase(write, access = manage_access(&id))]
    async fn delete_trigger(&self, id: TriggerId) -> DomainResult<()> {
        let trigger = self.trigger_repo().find_by_id(&id).await?;
        self.trigger_repo().delete(&trigger).await
    }

    #[usecase(read, access = Access::Requires(Permission::ManageTriggers(pipeline_id.clone())))]
    async fn list_pipeline_triggers(&self, pipeline_id: PipelineId) -> DomainResult<Vec<Trigger>> {
        self.trigger_repo().list_by_pipeline(&pipeline_id).await
    }

    /// The `runPipeline` of the caller on the trigger is the check. The fire then runs as the
    /// trigger-runner App.
    #[usecase(write, access = Access::Requires(Permission::RunTriggerPipeline(id.clone())))]
    async fn fire_trigger_now(&self, id: TriggerId) -> DomainResult<Job> {
        self.fire(&id).await
    }

    /// What a fire needs, read by the trigger firer service.
    #[usecase(read, access = Access::Authenticated)]
    async fn resolve_trigger_run(&self, id: TriggerId) -> DomainResult<TriggerRun> {
        service_only(self.ctx().caller())?;
        self.resolve_run(&id).await
    }

    /// The outcome of a fire, sent by the trigger firer as a service. Only the observation is
    /// written: an edit that committed during the run stays.
    #[usecase(write, access = manage_access(&id))]
    async fn record_trigger_fire(
        &self,
        id: TriggerId,
        status: &'static str,
    ) -> DomainResult<FireObservation> {
        let observation = FireObservation {
            fired_at: clock::now(),
            status: status.to_owned(),
        };
        self.trigger_repo().record_fire(&id, &observation).await?;
        Ok(observation)
    }

    /// One pass over the cron triggers that have no next fire time. A trigger whose next fire
    /// time cannot be computed is logged and stays unscheduled.
    #[usecase(write, access = Access::Authenticated)]
    async fn schedule_cron_triggers(&self) -> DomainResult<Vec<Trigger>> {
        service_only(self.ctx().caller())?;
        let now = clock::now();
        let rules = self.trigger_rules();
        self.trigger_repo()
            .seed_cron(&|trigger| rules.next_cron_fire(trigger, now))
            .await
    }

    /// One claim of the cron triggers due at `now`: the store advances each claimed row to its
    /// next fire time in the same transaction, thus no pass and no instance claims a row twice.
    #[usecase(write, access = Access::Authenticated)]
    async fn claim_due_triggers(
        &self,
        now: DateTime<Utc>,
        limit: i64,
    ) -> DomainResult<Vec<Trigger>> {
        service_only(self.ctx().caller())?;
        let rules = self.trigger_rules();
        self.trigger_repo()
            .claim_due_cron(now, limit, &|trigger| rules.next_cron_fire(trigger, now))
            .await
    }
}

/// A verified delivery. A ping is answered without a write. An event is recorded under its
/// dedupe key, then fired. The key is the delivery id, else the verified digest, thus each form of
/// one signature is one key.
pub enum VerifiedDelivery {
    Ping,
    Event {
        trigger_id: TriggerId,
        key: String,
        delivery_id: Option<String>,
        payload: Option<serde_json::Value>,
    },
}

/// The management of triggers does not give run rights, thus a create and an update also ask for
/// them.
fn create_trigger_access(pipeline_id: &PipelineId) -> Access {
    Access::RequiresAll(vec![
        Permission::ManageTriggers(pipeline_id.clone()),
        Permission::RunPipeline(pipeline_id.clone()),
    ])
}

fn update_trigger_access(id: &TriggerId) -> Access {
    Access::RequiresAll(vec![
        Permission::ManageTrigger(id.clone()),
        Permission::RunTriggerPipeline(id.clone()),
    ])
}

/// The access of a use case on one trigger, and of the record of a fire.
fn manage_access(id: &TriggerId) -> Access {
    Access::Requires(Permission::ManageTrigger(id.clone()))
}

fn trigger_not_found(id: &TriggerId) -> DomainError {
    DomainError::not_found("Trigger", id)
}

fn internal(e: &DomainError) -> DomainError {
    DomainError::internal(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_create_or_an_update_also_needs_the_run_right() {
        let created = create_trigger_access(&PipelineId::new("p1"));
        let updated = update_trigger_access(&TriggerId::new("t1"));

        assert_eq!(created.to_string(), "manageTriggers+runPipeline");
        assert_eq!(updated.to_string(), "manageTriggers+runPipeline");
        assert_eq!(
            manage_access(&TriggerId::new("t1")).to_string(),
            "manageTriggers"
        );
    }
}
