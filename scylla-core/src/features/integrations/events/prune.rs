//! The cleanup of the deliveries: a minute after the start, then each hour, a task deletes the
//! deliveries of the webhooks and the keys of the polls of more than 7 days.

use std::time::Duration;

use tokio::task::JoinHandle;
use tokio::time::{interval_at, Instant, MissedTickBehavior};
use tracing::Instrument;

use super::store;
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// The time after the start of the server before the first cleanup.
const FIRST: Duration = Duration::from_secs(60);

/// The time between two cleanups.
const TICK: Duration = Duration::from_secs(60 * 60);

/// Starts the task that deletes the old deliveries.
pub(super) fn spawn(ctx: IntegrationContext) -> JoinHandle<()> {
    let task = async move {
        let mut ticks = interval_at(Instant::now() + FIRST, TICK);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            if let Err(error) = prune(&ctx).await {
                tracing::warn!(%error, "the old deliveries of the webhooks stay");
            }
        }
    };
    tokio::spawn(task.in_current_span())
}

/// Deletes the deliveries of more than 7 days.
async fn prune(ctx: &IntegrationContext) -> Result<(), Error> {
    let mut tx = ctx.pool.begin().await?;
    let deleted = store::prune(&mut tx).await?;
    tx.commit().await?;
    if deleted > 0 {
        tracing::debug!(deleted, "old deliveries of the webhooks deleted");
    }
    Ok(())
}
