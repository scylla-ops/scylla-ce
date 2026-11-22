//! The task of the retention: each hour, it deletes the sessions, the one-time tokens and the
//! invitations whose use ended before their time in the settings.

use std::time::Duration;

use sqlx::PgPool;
use time::OffsetDateTime;
use tokio::task::JoinHandle;

use super::{store, RetentionSettings};
use crate::Error;

/// The time between two passes.
const TICK: Duration = Duration::from_secs(60 * 60);

/// Starts the task: each hour, it runs one pass.
pub(crate) fn spawn(pool: PgPool, settings: RetentionSettings) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + TICK, TICK);
        loop {
            ticks.tick().await;
            if let Err(error) = run(&pool, &settings).await {
                tracing::warn!(%error, "the old sessions, links and invitations stay");
            }
        }
    })
}

/// Runs one pass: deletes the closed sessions, the used or ended one-time tokens and the
/// accepted or ended invitations, each after its time in the settings.
pub async fn run(pool: &PgPool, settings: &RetentionSettings) -> Result<(), Error> {
    let now = OffsetDateTime::now_utc();
    let mut tx = pool.begin().await?;
    store::purge_sessions(&mut tx, now - settings.session_history).await?;
    store::purge_one_time_tokens(&mut tx, now - settings.one_time_token_retention).await?;
    store::purge_invitations(&mut tx, now - settings.invitation_retention).await?;
    tx.commit().await?;
    Ok(())
}
