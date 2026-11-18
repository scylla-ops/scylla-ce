//! The history of the fires of the triggers of a workspace: one page of the fires, and their
//! counts for a period in the time zone of the workspace.

use scylla_domain::projects::triggers::{
    FireId, FireStats, FireStatsQuery, ListFires, TriggerFire,
};
use scylla_domain::Page;
use sqlx::PgPool;

use super::store::{fire_figures, fires, schedules};
use crate::Error;

/// Returns one page of the fires of a query, newest due time first.
pub(super) async fn list(
    pool: &PgPool,
    query: ListFires,
) -> Result<Page<TriggerFire, FireId>, Error> {
    let mut tx = pool.begin().await?;
    let mut items = fires::list(&mut tx, &query).await?;
    tx.commit().await?;
    let size = query.page.size.get();
    let next_after = (items.len() > size).then(|| items[size - 1].id);
    items.truncate(size);
    Ok(Page { items, next_after })
}

/// Returns the counts of the fires of a query. Refuses more buckets than the most in the time
/// zone of the workspace.
pub(super) async fn stats(pool: &PgPool, query: FireStatsQuery) -> Result<FireStats, Error> {
    // Read the time zone of the workspace, and find the buckets of the period in it.
    let mut tx = pool.begin().await?;
    let zone = schedules::zone(&mut tx, query.fires.workspace).await?;
    let starts = match query.bucket {
        Some(span) => query
            .period
            .starts(zone, span)
            .map_err(|error| Error::invalid("bucket", error.to_string()))?,
        None => Vec::new(),
    };

    // Count the fires of the period: all of them, then by bucket and by group.
    let (fires, period) = (&query.fires, query.period);
    let totals = fire_figures::totals(&mut tx, fires, period).await?;
    let found = match starts.is_empty() {
        true => Vec::new(),
        false => fire_figures::buckets(&mut tx, fires, period, &starts).await?,
    };
    let groups = match query.group_by {
        Some(by) => fire_figures::groups(&mut tx, fires, period, by).await?,
        None => Vec::new(),
    };
    tx.commit().await?;
    Ok(FireStats {
        totals,
        buckets: FireStats::buckets(&starts, &found),
        groups: FireStats::top(groups, query.limit),
        time_zone: zone,
    })
}
