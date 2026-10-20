pub mod commands;
pub mod delivery;
pub mod fire;
pub mod queries;
pub mod repository;
pub mod schedule;
pub mod scheduler;
pub mod webhook;

pub use commands::{
    ClaimDueTriggers, CreateTrigger, CreatedTrigger, DeleteTrigger, NewTrigger, RecordTriggerFire,
    ScheduleCronTriggers, SetTriggerEnabled, UpdateTrigger,
};
pub use delivery::TriggerDeliveryRepository;
pub use fire::{FireTriggerNow, TriggerFireUseCases, TriggerFirer, TriggerFiring};
pub use queries::{GetTrigger, ListPipelineTriggers, ResolveTriggerRun, TriggerRun};
pub use repository::{NextFire, TriggerRepository};
pub use schedule::{CronSchedule, next_fire_time};
pub use scheduler::TriggerCronScheduler;
pub use webhook::{
    DEFAULT_SIGNATURE_HEADER, IngestOutcome, IngestWebhook, WebhookIngressUseCases, verified_digest,
};

use crate::application::{AppRepository, PipelineRepository, ProjectRepository, SecretCipher};
use crate::domain::app::{App, TRIGGER_RUNNER_APP_NAME};
use crate::domain::clock;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{AppId, OrganizationId};
use crate::domain::role::RoleName;
use crate::domain::trigger::Trigger;
use chrono::{DateTime, Utc};
use derive_more::Constructor;
use scylla_auth::authz::{Grant, ORGANIZATION_TRIGGER_RUNNER_ROLE, Principal, Scope};
use std::sync::Arc;
use tracing::warn;

/// The trigger aggregate's stage runners, one block per action in `commands.rs` and
/// `queries.rs`. It has no public method; `Actions::run` drives it, also for the reads and the
/// writes of the cron scheduler and of the trigger firer.
#[derive(Constructor)]
pub struct TriggerUseCases {
    pub(super) trigger_repo: Arc<dyn TriggerRepository>,
    pub(super) pipeline_repo: Arc<dyn PipelineRepository>,
    pub(super) project_repo: Arc<dyn ProjectRepository>,
    pub(super) app_repo: Arc<dyn AppRepository>,
    /// Reversible: HMAC verification needs the plaintext back.
    pub(super) cipher: Arc<dyn SecretCipher>,
    pub(super) schedule: Arc<dyn CronSchedule>,
}

impl TriggerUseCases {
    fn schedule_next(&self, trigger: &mut Trigger) -> DomainResult<()> {
        let next = next_fire_time(trigger, &*self.schedule, clock::now())?;
        trigger.set_next_fire_at(next);
        Ok(())
    }

    fn next_cron_fire(
        &self,
        trigger: &Trigger,
        after: DateTime<Utc>,
    ) -> DomainResult<DateTime<Utc>> {
        next_fire_time(trigger, &*self.schedule, after)
            .and_then(|next| {
                next.ok_or_else(|| DomainError::internal("non-cron trigger reached the cron pass"))
            })
            .inspect_err(|e| {
                warn!(trigger_id = %trigger.id(), error = %e, "cron: no next fire time; trigger will not fire");
            })
    }

    async fn trigger_runner(&self, trigger: &Trigger) -> DomainResult<AppId> {
        let pipeline = self.pipeline_repo.find_by_id(trigger.pipeline_id()).await?;
        let project = self.project_repo.find_by_id(pipeline.project_id()).await?;
        self.app_repo
            .find_trigger_runner(project.organization_id())
            .await?
            .ok_or_else(|| {
                DomainError::internal("trigger-runner App is not provisioned for this organization")
            })
    }

    async fn ensure_runner_app(&self, organization_id: &OrganizationId) -> DomainResult<()> {
        if self
            .app_repo
            .find_trigger_runner(organization_id)
            .await?
            .is_some()
        {
            return Ok(());
        }

        let app = App::trigger_runner(organization_id.clone())?;
        let grant = Grant::new(
            Principal::App(app.id().clone()),
            RoleName::new(ORGANIZATION_TRIGGER_RUNNER_ROLE)?,
            Scope::Organization(organization_id.clone()),
        );

        match self.app_repo.provision(&app, &grant).await {
            Ok(()) => Ok(()),
            Err(DomainError::Conflict(_)) => {
                match self.app_repo.find_trigger_runner(organization_id).await? {
                    // Lost the race to a concurrent first-create.
                    Some(_) => Ok(()),
                    None => Err(DomainError::conflict(format!(
                        "App name '{TRIGGER_RUNNER_APP_NAME}' is reserved for the trigger runner; delete the App that holds it"
                    ))),
                }
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests;
