//! The cron schedules of the triggers: their check, their times in the time zone of the
//! workspace, and the loop that fires the triggers that are due in each server.

use std::collections::VecDeque;
use std::str::FromStr;
use std::time::Duration;

use croner::Cron;
use scylla_domain::projects::jobs::JobOrigin;
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::triggers::{FireSource, TriggerId, FIRE_RETENTION_DAYS};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::TimeZone;
use scylla_extension::Tx;
use time::OffsetDateTime;
use tokio::task::JoinHandle;

use super::fire::{self, Call};
use super::store::{deliveries, fires, schedules, triggers};
use crate::features::activity;
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// The time between two rounds of the loop.
const TICK: Duration = Duration::from_secs(15);

/// The most triggers that one round takes.
const BATCH: i64 = 100;

/// The time that the keys of the deliveries of the webhooks stay.
const DELIVERY_RETENTION: time::Duration = time::Duration::days(7);

/// The time that the history keeps a fire.
const FIRE_RETENTION: time::Duration = time::Duration::days(FIRE_RETENTION_DAYS);

/// The most missed times that one round records for one trigger: the newest ones.
const MAX_MISSED: usize = 1000;

/// The most wall times in a row that do not fire before the search of a next time stops.
const MAX_SKIPPED: usize = 10_000;

/// Checks a schedule: exactly 5 cron fields.
pub(crate) fn check(schedule: &str) -> Result<Cron, Error> {
    let invalid = |reason: &str| Error::invalid("cron.schedule", reason);
    if schedule.split_whitespace().count() != 5 || schedule.contains('@') {
        return Err(invalid(
            "must have 5 fields: minute, hour, day, month, weekday",
        ));
    }
    Cron::from_str(schedule).map_err(|error| invalid(&error.to_string()))
}

/// Returns the first time of a schedule after `after` in a time zone, in whole seconds, or `None`
/// if the schedule has no next time.
pub(crate) fn next(
    schedule: &str,
    zone: TimeZone,
    after: OffsetDateTime,
) -> Option<OffsetDateTime> {
    times(schedule, zone, after).next()
}

/// Returns the times of a schedule after `after` and up to `until` in a time zone, oldest first,
/// at most `max`.
pub(crate) fn between(
    schedule: &str,
    zone: TimeZone,
    after: OffsetDateTime,
    until: OffsetDateTime,
    max: usize,
) -> Vec<OffsetDateTime> {
    times(schedule, zone, after)
        .take_while(|at| *at <= until)
        .take(max)
        .collect()
}

/// Returns the times of a schedule after `after` in a time zone, oldest first. A wall time that
/// the change to summer time skips does not fire. A wall time that the change to winter time
/// shows two times fires one time, at its first occurrence.
fn times(
    schedule: &str,
    zone: TimeZone,
    after: OffsetDateTime,
) -> impl Iterator<Item = OffsetDateTime> {
    let cron = Cron::from_str(schedule).ok();
    let mut wall = zone.wall_time(after);
    let mut last = after;
    std::iter::from_fn(move || {
        let cron = cron.as_ref()?;
        for _ in 0..MAX_SKIPPED {
            wall = cron
                .find_next_occurrence(&wall.and_utc(), false)
                .ok()?
                .naive_utc();
            if let Some(at) = zone.first_instant(wall).filter(|at| *at > last) {
                last = at;
                return Some(at);
            }
        }
        None
    })
}

/// Returns the times of a schedule after its due time and up to `now` that the server missed: at
/// most `MAX_MISSED`, the newest ones, and none that the history would not keep.
fn missed(
    schedule: &str,
    zone: TimeZone,
    due_at: OffsetDateTime,
    now: OffsetDateTime,
) -> Vec<OffsetDateTime> {
    let mut found = VecDeque::new();
    for at in times(schedule, zone, due_at.max(now - FIRE_RETENTION)).take_while(|at| *at <= now) {
        if found.len() == MAX_MISSED {
            found.pop_front();
        }
        found.push_back(at);
    }
    found.into()
}

/// Records the missed times of a trigger in the activity feed, if it has some.
async fn record_missed(
    tx: &mut Tx<'_>,
    id: TriggerId,
    times: &[OffsetDateTime],
) -> Result<(), Error> {
    if times.is_empty() {
        return Ok(());
    }
    match triggers::find(tx, id).await? {
        Some(trigger) => activity::time_missed(tx, &trigger, times).await,
        None => Ok(()),
    }
}

/// Returns the next time after now of a schedule in the time zone of a workspace.
pub(crate) async fn next_in(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    schedule: &str,
) -> Result<Option<OffsetDateTime>, Error> {
    let zone = schedules::zone(tx, workspace).await?;
    Ok(next(schedule, zone, OffsetDateTime::now_utc()))
}

/// Returns the next time after now of a schedule in the time zone of the workspace of a pipeline.
pub(crate) async fn next_for(
    tx: &mut Tx<'_>,
    pipeline: PipelineId,
    schedule: &str,
) -> Result<Option<OffsetDateTime>, Error> {
    let zone = schedules::pipeline_zone(tx, pipeline).await?;
    Ok(next(schedule, zone, OffsetDateTime::now_utc()))
}

/// Computes again the next time of each enabled cron trigger of a workspace in its new time zone.
pub(crate) async fn reschedule(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    zone: TimeZone,
) -> Result<(), Error> {
    let now = OffsetDateTime::now_utc();
    for (id, schedule) in schedules::crons(tx, workspace).await? {
        schedules::set_next(tx, id, next(&schedule, zone, now)).await?;
    }
    Ok(())
}

/// Starts the loop: each 15 seconds, it fires the cron triggers that are due and forgets the
/// old deliveries of the webhooks and the old fires.
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

/// Fires the cron triggers that are due, one time each at their due time, and returns their
/// count. Each trigger gets its next time in the same transaction, so a second server skips it.
/// A missed time does not fire later: the history records it as missed, and the activity feed
/// records one row for the missed times of a trigger. A trigger of an archived pipeline, or of a
/// file pipeline whose file left the default branch, gets its next time and does not fire. A file
/// pipeline runs its file at the head of the default branch.
pub(crate) async fn tick(vcs: &IntegrationContext) -> Result<usize, Error> {
    // Take the due triggers, move each to its next time, and record the times that it missed.
    let now = OffsetDateTime::now_utc();
    let mut tx = vcs.pool.begin().await?;
    let due = schedules::due(&mut tx, BATCH).await?;
    for trigger in &due {
        let after = next(&trigger.schedule, trigger.zone, now);
        schedules::set_next(&mut tx, trigger.id, after).await?;
        if trigger.active {
            let times = missed(&trigger.schedule, trigger.zone, trigger.due_at, now);
            fires::insert_missed(&mut tx, trigger.id, &times).await?;
            record_missed(&mut tx, trigger.id, &times).await?;
        }
    }
    deliveries::purge(&mut tx, now - DELIVERY_RETENTION).await?;
    fires::purge(&mut tx, now - FIRE_RETENTION).await?;
    tx.commit().await?;

    // Fire each one of an active pipeline. A failed fire is recorded on its trigger.
    let active: Vec<_> = due.iter().filter(|trigger| trigger.active).collect();
    for trigger in &active {
        let call = Call {
            source: FireSource::Schedule,
            due_at: trigger.due_at,
        };
        if let Err(error) = fire::by_id(vcs, trigger.id, JobOrigin::Cron, call).await {
            tracing::warn!(%error, trigger = %trigger.id, "a cron trigger did not fire");
        }
    }
    Ok(active.len())
}
