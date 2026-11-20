//! The task of the polls: each 2 seconds, it takes the links whose poll is due and polls each
//! one.

use std::time::Duration;

use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{interval_at, Instant, MissedTickBehavior};
use tracing::Instrument;

use super::poll::{self, SAFETY_SECONDS};
use super::store::links;
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// The time after the start of the server before the first look for the due links.
const FIRST: Duration = TICK;

/// The time between two looks for the due links.
const TICK: Duration = Duration::from_secs(2);

/// The most links of one look.
const BATCH: i64 = 20;

/// Starts the task of the polls.
pub(super) fn spawn(ctx: IntegrationContext) -> JoinHandle<()> {
    let task = async move {
        let mut ticks = interval_at(Instant::now() + FIRST, TICK);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            if let Err(error) = pass(&ctx).await {
                tracing::warn!(%error, "the polls of the repositories wait");
            }
        }
    };
    tokio::spawn(task.in_current_span())
}

/// Takes the due links, and polls each one.
async fn pass(ctx: &IntegrationContext) -> Result<(), Error> {
    // Take the due links, and move their next poll.
    let mut tx = ctx.pool.begin().await?;
    let due = links::claim(&mut tx, BATCH, SAFETY_SECONDS).await?;
    tx.commit().await?;

    // Poll each link.
    let mut polls = JoinSet::new();
    for project in due {
        let ctx = ctx.clone();
        polls.spawn(async move { (project, poll::poll(&ctx, project).await) }.in_current_span());
    }
    while let Some(done) = polls.join_next().await {
        match done {
            Ok((_, Ok(()))) => {}
            Ok((project, Err(error))) => tracing::warn!(%project, %error, "a poll failed"),
            Err(error) => tracing::warn!(%error, "a poll stopped"),
        }
    }
    Ok(())
}
