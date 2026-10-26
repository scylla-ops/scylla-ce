use crate::access::{CallerOf, Permissions};
use scylla_core::application::actions::service_only;
use scylla_core::application::pipeline::RunPipelineWithInputs;
use scylla_core::application::trigger::commands::generate_webhook_secret;
use scylla_core::application::trigger::fire::resolve_inputs;
use scylla_core::application::trigger::{
    ClaimDueTriggers, CreateTrigger, CreatedTrigger, CronSchedule, DeleteTrigger, FireTriggerNow,
    GetTrigger, ListPipelineTriggers, RecordTriggerFire, ResolveTriggerRun, ScheduleCronTriggers,
    SetTriggerEnabled, TriggerRepository, TriggerRun, TriggerUseCases, UpdateTrigger,
};
use scylla_core::application::{
    AppRepository, PipelineRepository, ProjectRepository, SecretCipher,
};
use scylla_domain::domain::caller::{CallerContext, ServiceIdentity};
use scylla_domain::domain::clock;
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::TriggerId;
use scylla_domain::domain::job::{Job, JobOrigin};
use scylla_domain::domain::trigger::{FireObservation, Trigger, TriggerSource};
use scylla_extension::Describe;
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

    fn trigger_rules(&self) -> TriggerUseCases {
        TriggerUseCases::new(
            self.trigger_repo().clone(),
            self.trigger_pipelines().clone(),
            self.trigger_projects().clone(),
            self.trigger_apps().clone(),
            self.trigger_cipher().clone(),
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

    async fn run_as_runner(&self, trigger: &Trigger, runner: CallerContext) -> DomainResult<Job> {
        let origin = match trigger.source() {
            TriggerSource::Cron(_) => JobOrigin::Cron {
                trigger_id: trigger.id().clone(),
            },
            TriggerSource::Webhook(_) => JobOrigin::Webhook {
                trigger_id: trigger.id().clone(),
                delivery_id: None,
            },
        };
        let run = RunPipelineWithInputs {
            id: trigger.pipeline_id().clone(),
            inputs: resolve_inputs(trigger, None)?,
            origin,
        };
        self.require_as(&runner, run.access()).await?;
        let job = self.new_job(&run.id, run.origin, run.inputs).await?;
        self.start(&job).await
    }

    async fn record_as(
        &self,
        caller: &CallerContext,
        record: RecordTriggerFire,
    ) -> DomainResult<()> {
        self.require_as(caller, record.access()).await?;
        let observation = FireObservation {
            fired_at: clock::now(),
            status: record.status.to_owned(),
        };
        self.trigger_repo()
            .record_fire(&record.id, &observation)
            .await
    }

    async fn fire(&self, id: &TriggerId) -> DomainResult<Job> {
        let service = CallerContext::Service(ServiceIdentity::trigger_firer());
        let TriggerRun { trigger, runner } = self.resolve_run(id).await?;
        let outcome = match runner {
            Ok(runner) => {
                self.run_as_runner(&trigger, CallerContext::App(runner))
                    .await
            }
            Err(e) => Err(e),
        };
        let record = RecordTriggerFire {
            id: id.clone(),
            status: if outcome.is_ok() { "ok" } else { "error" },
        };
        if let Err(e) = self.record_as(&service, record).await {
            tracing::warn!(trigger_id = %id, error = %e, "failed to record trigger fire status");
        }
        outcome
    }

    #[usecase(write, access = cmd.access())]
    async fn create_trigger(&self, cmd: CreateTrigger) -> DomainResult<CreatedTrigger> {
        let rules = self.trigger_rules();
        let pipeline = self
            .trigger_pipelines()
            .find_by_id(&cmd.pipeline_id)
            .await?;
        let project = self
            .trigger_projects()
            .find_by_id(pipeline.project_id())
            .await?;
        let mut trigger = Trigger::create(cmd.pipeline_id, cmd.name, cmd.source, cmd.inputs)?;
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

    #[usecase(read, access = query.access())]
    async fn get_trigger(&self, query: GetTrigger) -> DomainResult<Trigger> {
        self.trigger_repo().find_by_id(&query.id).await
    }

    #[usecase(write, access = cmd.access())]
    async fn update_trigger(&self, cmd: UpdateTrigger) -> DomainResult<Trigger> {
        let mut trigger = self.trigger_repo().find_by_id(&cmd.id).await?;
        trigger.update(cmd.name, cmd.source, cmd.inputs)?;
        self.trigger_rules().schedule_next(&mut trigger)?;
        self.trigger_repo().update(&trigger).await
    }

    #[usecase(write, access = cmd.access())]
    async fn set_trigger_enabled(&self, cmd: SetTriggerEnabled) -> DomainResult<Trigger> {
        let mut trigger = self.trigger_repo().find_by_id(&cmd.id).await?;
        if cmd.enabled {
            trigger.enable();
            self.trigger_rules().schedule_next(&mut trigger)?;
        } else {
            trigger.disable();
        }
        self.trigger_repo().update(&trigger).await
    }

    #[usecase(write, access = cmd.access())]
    async fn delete_trigger(&self, cmd: DeleteTrigger) -> DomainResult<()> {
        let trigger = self.trigger_repo().find_by_id(&cmd.id).await?;
        self.trigger_repo().delete(&trigger).await
    }

    #[usecase(read, access = query.access())]
    async fn list_pipeline_triggers(
        &self,
        query: ListPipelineTriggers,
    ) -> DomainResult<Vec<Trigger>> {
        self.trigger_repo()
            .list_by_pipeline(&query.pipeline_id)
            .await
    }

    #[usecase(write, access = cmd.access())]
    async fn fire_trigger_now(&self, cmd: FireTriggerNow) -> DomainResult<Job> {
        self.fire(&cmd.id).await
    }

    #[usecase(read, access = query.access())]
    async fn resolve_trigger_run(&self, query: ResolveTriggerRun) -> DomainResult<TriggerRun> {
        service_only(self.ctx().caller())?;
        self.resolve_run(&query.id).await
    }

    #[usecase(write, access = cmd.access())]
    async fn record_trigger_fire(&self, cmd: RecordTriggerFire) -> DomainResult<FireObservation> {
        let observation = FireObservation {
            fired_at: clock::now(),
            status: cmd.status.to_owned(),
        };
        self.trigger_repo()
            .record_fire(&cmd.id, &observation)
            .await?;
        Ok(observation)
    }

    #[usecase(write, access = cmd.access())]
    #[allow(unused_variables)]
    async fn schedule_cron_triggers(
        &self,
        cmd: ScheduleCronTriggers,
    ) -> DomainResult<Vec<Trigger>> {
        service_only(self.ctx().caller())?;
        let now = clock::now();
        let rules = self.trigger_rules();
        self.trigger_repo()
            .seed_cron(&|trigger| rules.next_cron_fire(trigger, now))
            .await
    }

    #[usecase(write, access = cmd.access())]
    async fn claim_due_triggers(&self, cmd: ClaimDueTriggers) -> DomainResult<Vec<Trigger>> {
        service_only(self.ctx().caller())?;
        let rules = self.trigger_rules();
        let now = cmd.now;
        self.trigger_repo()
            .claim_due_cron(now, cmd.limit, &|trigger| {
                rules.next_cron_fire(trigger, now)
            })
            .await
    }
}
