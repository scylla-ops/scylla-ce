use crate::application::trigger::fire::TriggerFiring;
use crate::application::trigger::{ClaimDueTriggers, ScheduleCronTriggers, TriggerUseCases};
use crate::domain::caller::{CallerContext, ServiceIdentity};
use crate::domain::clock;
use scylla_extension::Actions;
use std::sync::Arc;
use tracing::{info, instrument, warn};

const CRON_CLAIM_BATCH: i64 = 100;

/// Missed occurrences during downtime are skipped, not backfilled.
pub struct TriggerCronScheduler {
    actions: Arc<Actions>,
    triggers: Arc<TriggerUseCases>,
    firing: Arc<dyn TriggerFiring>,
}

impl TriggerCronScheduler {
    #[must_use]
    pub fn new(
        actions: Arc<Actions>,
        triggers: Arc<TriggerUseCases>,
        firing: Arc<dyn TriggerFiring>,
    ) -> Self {
        Self {
            actions,
            triggers,
            firing,
        }
    }

    #[instrument(skip(self))]
    pub async fn tick(&self) -> usize {
        let caller = CallerContext::Service(ServiceIdentity::cron_scheduler());
        self.seed_unscheduled(&caller).await;
        self.fire_due(&caller).await
    }

    async fn seed_unscheduled(&self, caller: &CallerContext) {
        let seed = ScheduleCronTriggers;
        if let Err(e) = self.actions.run(&*self.triggers, caller, seed).await {
            warn!(error = %e, "cron seed: could not seed unscheduled triggers");
        }
    }

    async fn fire_due(&self, caller: &CallerContext) -> usize {
        let claim = ClaimDueTriggers {
            now: clock::now(),
            limit: CRON_CLAIM_BATCH,
        };
        let due = match self.actions.run(&*self.triggers, caller, claim).await {
            Ok(due) => due,
            Err(e) => {
                warn!(error = %e, "cron claim: could not claim due triggers");
                return 0;
            }
        };

        let mut fired = 0usize;
        for trigger in due {
            match self.firing.fire(trigger.id(), None, None).await {
                Ok(job) => {
                    info!(trigger_id = %trigger.id(), job_id = %job.id(), "cron trigger fired");
                    fired += 1;
                }
                // The occurrence is already consumed; one bad trigger must not stall the rest.
                Err(e) => {
                    warn!(trigger_id = %trigger.id(), error = %e, "cron trigger fire failed");
                }
            }
        }
        if fired > 0 {
            info!(fired, "cron scheduler fired due triggers");
        }
        fired
    }
}
