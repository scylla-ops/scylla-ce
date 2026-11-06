//! The delete of the old notifications.

use std::time::Duration;

use sqlx::PgPool;
use time::OffsetDateTime;

use super::store::{events, inbox};
use crate::Error;

/// The most rows that one person keeps.
const MAX_ROWS: i64 = 1000;

/// The time that a fact stays after its dispatch.
const EVENT_RETENTION: time::Duration = time::Duration::days(30);

/// Closes the rows of the invitations that ended, deletes the rows older than `retention` and the
/// oldest rows above the limit of a person, and deletes the old facts.
pub async fn run(pool: &PgPool, retention: Duration) -> Result<(), Error> {
    let now = OffsetDateTime::now_utc();
    let mut tx = pool.begin().await?;
    inbox::close_ended_invitations(&mut tx).await?;
    let old = inbox::purge_older(&mut tx, now - retention).await?;
    let above = inbox::purge_above(&mut tx, MAX_ROWS).await?;
    let facts = events::purge(&mut tx, now - EVENT_RETENTION).await?;
    tx.commit().await?;
    if old + above + facts > 0 {
        tracing::info!(event = "notifications.purged", old, above, facts);
    }
    Ok(())
}
