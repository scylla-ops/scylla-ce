//! The summaries of the triggers of a workspace for the dashboard: one transaction reads the time
//! zone of the workspace and the triggers, sorts them, and adds to the triggers of the page their
//! next fires and their recent results.

use scylla_domain::projects::triggers::{
    ListWorkspaceTriggers, TriggerId, TriggerKind, TriggerSummary, MAX_UPCOMING_FIRES,
};
use scylla_domain::{Page, TimeZone};
use sqlx::PgPool;
use time::OffsetDateTime;

use super::schedule;
use super::store::{fires, schedules, summaries};
use crate::Error;

/// Returns one page of the summaries of a query at the time `now`, with the time zone of the
/// workspace.
pub(super) async fn list(
    pool: &PgPool,
    query: ListWorkspaceTriggers,
    now: OffsetDateTime,
) -> Result<(Page<TriggerSummary, TriggerId>, TimeZone), Error> {
    // Read the time zone and the triggers, then keep one page.
    let mut tx = pool.begin().await?;
    let zone = schedules::zone(&mut tx, query.workspace).await?;
    let all = summaries::all(&mut tx, &query).await?;
    let mut page = query.order.page(all, query.page);

    // Read the results of the triggers of the page.
    let ids: Vec<TriggerId> = page
        .items
        .iter()
        .map(|summary| summary.trigger.id)
        .collect();
    let mut jobs = summaries::last_jobs(&mut tx, &ids).await?;
    let mut last = fires::last_of(&mut tx, &ids).await?;
    let problems = fires::problems(&mut tx, &ids, query.since).await?;
    tx.commit().await?;

    // Add the next fires and the results to each summary.
    for summary in &mut page.items {
        let id = summary.trigger.id;
        summary.last_job_status = jobs.remove(&id);
        summary.last_fire = last.remove(&id);
        (summary.failed_fires, summary.missed_times) =
            problems.get(&id).copied().unwrap_or_default();
        summary.upcoming = upcoming(summary, zone, now, query.until);
    }
    Ok((page, zone))
}

/// Returns the fires of a summary after `now` and up to `until`: none for a trigger with no next
/// fire.
fn upcoming(
    summary: &TriggerSummary,
    zone: TimeZone,
    now: OffsetDateTime,
    until: Option<OffsetDateTime>,
) -> Vec<OffsetDateTime> {
    match (&summary.trigger.kind, summary.next_fire(), until) {
        (TriggerKind::Cron(text), Some(_), Some(until)) => {
            schedule::between(text, zone, now, until, MAX_UPCOMING_FIRES)
        }
        _ => Vec::new(),
    }
}
