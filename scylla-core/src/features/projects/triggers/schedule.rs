//! The cron schedules of the triggers: their check, their next time, and the loop that fires the
//! triggers that are due in each server.

use std::str::FromStr;
use std::time::Duration;

use croner::Cron;
use time::OffsetDateTime;
use tokio::task::JoinHandle;

use super::fire;
use super::store::{deliveries, triggers};
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// The time between two rounds of the loop.
const TICK: Duration = Duration::from_secs(15);

/// The most triggers that one round takes.
const BATCH: i64 = 100;

/// The time that the keys of the deliveries of the webhooks stay.
const DELIVERY_RETENTION: time::Duration = time::Duration::days(7);

/// Checks a schedule: exactly 5 cron fields, in UTC.
pub(crate) fn check(schedule: &str) -> Result<Cron, Error> {
    let invalid = |reason: &str| Error::invalid("cron.schedule", reason);
    if schedule.split_whitespace().count() != 5 || schedule.contains('@') {
        return Err(invalid(
            "must have 5 fields: minute, hour, day, month, weekday",
        ));
    }
    Cron::from_str(schedule).map_err(|error| invalid(&error.to_string()))
}

/// Returns the first time of a schedule after `after`, in whole seconds, or `None` if the
/// schedule has no next time.
pub(crate) fn next(schedule: &str, after: OffsetDateTime) -> Option<OffsetDateTime> {
    let cron = Cron::from_str(schedule).ok()?;
    let start = chrono::DateTime::from_timestamp(after.unix_timestamp(), 0)?;
    let next = cron.find_next_occurrence(&start, false).ok()?;
    OffsetDateTime::from_unix_timestamp(next.timestamp()).ok()
}

/// Starts the loop: each 15 seconds, it fires the cron triggers that are due and forgets the
/// old deliveries of the webhooks.
pub(crate) fn spawn(vcs: IntegrationContext) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + TICK, TICK);
        loop {
            ticks.tick().await;
            if let Err(error) = tick(&vcs).await {
                tracing::warn!(%error, "the cron triggers did not fire");
            }
        }
    })
}

/// Fires the cron triggers that are due, one time each, and returns their count. Each trigger
/// gets its next time in the same transaction, so a second server skips it. A missed time does
/// not fire later. A trigger of an archived pipeline, or of a file pipeline whose file left the
/// default branch, gets its next time and does not fire. A file pipeline runs its file at the
/// head of the default branch.
pub(crate) async fn tick(vcs: &IntegrationContext) -> Result<usize, Error> {
    // Take the due triggers and move each to its next time.
    let now = OffsetDateTime::now_utc();
    let mut tx = vcs.pool.begin().await?;
    let due = triggers::due(&mut tx, BATCH).await?;
    for (id, schedule, _) in &due {
        triggers::set_next(&mut tx, *id, next(schedule, now)).await?;
    }
    deliveries::purge(&mut tx, now - DELIVERY_RETENTION).await?;
    tx.commit().await?;

    // Fire each one of an active pipeline. A failed fire is recorded on its trigger.
    let active: Vec<_> = due.iter().filter(|(_, _, active)| *active).collect();
    for (id, _, _) in &active {
        if let Err(error) = fire::by_id(vcs, *id).await {
            tracing::warn!(%error, trigger = %id, "a cron trigger did not fire");
        }
    }
    Ok(active.len())
}
