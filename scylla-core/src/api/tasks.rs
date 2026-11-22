//! The background tasks of the edition: the server runs each one at its start, then at its period,
//! until its stop.

use scylla_extension::{ScheduledTask, TaskContext};
use sqlx::PgPool;
use tokio::task::JoinHandle;
use tokio::time::{self, MissedTickBehavior};

/// Starts each task of the edition. A failed run goes to the log, and the next run comes at the
/// next period.
pub(crate) fn spawn(pool: &PgPool, tasks: &[ScheduledTask]) -> Vec<JoinHandle<()>> {
    tasks
        .iter()
        .cloned()
        .map(|scheduled| {
            let ctx = TaskContext::new(pool.clone());
            tokio::spawn(async move {
                let mut ticks = time::interval(scheduled.every);
                ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
                loop {
                    ticks.tick().await;
                    if let Err(error) = scheduled.task.run(&ctx).await {
                        tracing::warn!(task = scheduled.name, %error, "the task failed");
                    }
                }
            })
        })
        .collect()
}
