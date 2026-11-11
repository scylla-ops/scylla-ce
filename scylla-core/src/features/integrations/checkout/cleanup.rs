//! The cleanup of the GitLab deploy tokens of the checkouts: each few seconds, a task deletes on
//! GitLab the tokens that no job needs, then their rows.

use std::time::Duration;

use time::OffsetDateTime;
use tokio::task::JoinHandle;
use tokio::time::{interval, MissedTickBehavior};
use tracing::Instrument;

use super::gitlab::{self, Deleted};
use super::store;
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// The time between two looks for the tokens that no job needs.
const TICK: Duration = Duration::from_secs(5);

/// The most tokens of one look.
const BATCH: i64 = 50;

/// Starts the task that deletes the deploy tokens that no job needs.
pub(super) fn spawn(ctx: IntegrationContext) -> JoinHandle<()> {
    let task = async move {
        let mut ticks = interval(TICK);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut limited: Option<OffsetDateTime> = None;
        loop {
            ticks.tick().await;
            if limited.is_some_and(|until| OffsetDateTime::now_utc() < until) {
                continue;
            }
            limited = match sweep(&ctx).await {
                Ok(until) => until,
                Err(error) => {
                    tracing::warn!(%error, "the deploy tokens of the checkouts were not read");
                    None
                }
            };
        }
    };
    tokio::spawn(task.in_current_span())
}

/// Deletes the deploy tokens of the jobs that ended, that no agent holds or that were deleted,
/// and the tokens that expired. A token that GitLab does not delete stays until its expiry.
/// Returns the end of a limit of GitLab.
async fn sweep(ctx: &IntegrationContext) -> Result<Option<OffsetDateTime>, Error> {
    // Read the tokens that no job needs.
    let mut tx = ctx.pool.begin().await?;
    let tokens = store::unused(&mut tx, BATCH).await?;
    tx.commit().await?;

    // Delete each token on GitLab, then its row.
    for token in tokens {
        let gone = match gitlab::delete(ctx, &token).await {
            Deleted::Gone => true,
            Deleted::Limited(until) => return Ok(Some(until)),
            Deleted::Failed(reason) => {
                let id = token.id;
                tracing::warn!(token = %id, reason, "a deploy token of a checkout was not deleted");
                token.expires_at <= OffsetDateTime::now_utc()
            }
        };
        if gone {
            let mut tx = ctx.pool.begin().await?;
            store::delete(&mut tx, token.id).await?;
            tx.commit().await?;
        }
    }
    Ok(None)
}
