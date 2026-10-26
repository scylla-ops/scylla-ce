pub mod delivery;
pub mod fire;
pub mod repository;
pub mod schedule;
pub mod webhook;

pub use delivery::TriggerDeliveryRepository;
pub use repository::{NextFire, TriggerRepository};
pub use schedule::{CronSchedule, next_fire_time};
pub use webhook::{DEFAULT_SIGNATURE_HEADER, IngestOutcome, verified_digest};

use crate::application::{AppRepository, PipelineRepository, ProjectRepository};
use crate::domain::app::{App, TRIGGER_RUNNER_APP_NAME};
use crate::domain::clock;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{AppId, OrganizationId};
use crate::domain::role::RoleName;
use crate::domain::trigger::Trigger;
use chrono::{DateTime, Utc};
use scylla_auth::authz::{Grant, ORGANIZATION_TRIGGER_RUNNER_ROLE, Principal, Scope};
use std::sync::Arc;
use tracing::warn;
use uuid::Uuid;

/// The outcome of `create_trigger`. No `Debug`: `webhook_secret` is the plaintext that the caller
/// sees once.
pub struct CreatedTrigger {
    pub trigger: Trigger,
    pub webhook_secret: Option<String>,
}

/// What a fire needs: the trigger, refused when it is disabled, and the trigger-runner App of its
/// organization. A runner that cannot be found is a failed run, not a refusal, thus the fire still
/// records it.
pub struct TriggerRun {
    pub trigger: Trigger,
    pub runner: DomainResult<AppId>,
}

pub fn generate_webhook_secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// The trigger rules that the Triggers module of strata reads: the next fire time and the runner
/// app of an organization.
pub struct TriggerUseCases {
    pub(super) pipeline_repo: Arc<dyn PipelineRepository>,
    pub(super) project_repo: Arc<dyn ProjectRepository>,
    pub(super) app_repo: Arc<dyn AppRepository>,
    pub(super) schedule: Arc<dyn CronSchedule>,
}

impl TriggerUseCases {
    pub fn new(
        pipeline_repo: Arc<dyn PipelineRepository>,
        project_repo: Arc<dyn ProjectRepository>,
        app_repo: Arc<dyn AppRepository>,
        schedule: Arc<dyn CronSchedule>,
    ) -> Self {
        Self {
            pipeline_repo,
            project_repo,
            app_repo,
            schedule,
        }
    }

    pub fn schedule_next(&self, trigger: &mut Trigger) -> DomainResult<()> {
        let next = next_fire_time(trigger, &*self.schedule, clock::now())?;
        trigger.set_next_fire_at(next);
        Ok(())
    }

    pub fn next_cron_fire(
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

    pub async fn trigger_runner(&self, trigger: &Trigger) -> DomainResult<AppId> {
        let pipeline = self.pipeline_repo.find_by_id(trigger.pipeline_id()).await?;
        let project = self.project_repo.find_by_id(pipeline.project_id()).await?;
        self.app_repo
            .find_trigger_runner(project.organization_id())
            .await?
            .ok_or_else(|| {
                DomainError::internal("trigger-runner App is not provisioned for this organization")
            })
    }

    pub async fn ensure_runner_app(&self, organization_id: &OrganizationId) -> DomainResult<()> {
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
