//! The busy slots of the agents of a workspace for the dashboard: one transaction reads the time
//! zone of the workspace, then the busy time of the jobs in each part of the period.

use scylla_domain::identity::agents::{self, SlotQuery, SlotUsage};
use sqlx::PgPool;

use super::store::{figures, slots};
use crate::Error;

/// Returns the busy slots of a query, with a capacity of 0. Refuses more buckets than the most in
/// the time zone of the workspace.
pub(crate) async fn usage(pool: &PgPool, query: SlotQuery) -> Result<SlotUsage, Error> {
    // Read the time zone of the workspace, and find the buckets of the period in it.
    let mut tx = pool.begin().await?;
    let zone = figures::time_zone(&mut tx, query.workspace).await?;
    let starts = match query.bucket {
        Some(span) => query
            .period
            .starts(zone, span)
            .map_err(|error| Error::invalid("bucket", error.to_string()))?,
        None => Vec::new(),
    };

    // Add the busy time of the jobs in each part of the period.
    let bounds = agents::bounds(query.period, &starts);
    let found = slots::busy(&mut tx, query.workspace, query.agent, &bounds, query.end()).await?;
    tx.commit().await?;
    Ok(SlotUsage::of_parts(&query, &starts, &bounds, &found, zone))
}
