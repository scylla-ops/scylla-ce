//! The counts of the webhook deliveries of a workspace for a period, in the time zone of the
//! workspace: the deliveries of its connections and the fires of its webhook triggers.

use scylla_domain::integrations::deliveries::{DeliveryStats, DeliveryStatsQuery};

use super::store::deliveries;
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// Returns the delivery counts of a query. Refuses more buckets than the most in the time zone
/// of the workspace.
pub(super) async fn delivery_stats(
    ctx: &IntegrationContext,
    query: DeliveryStatsQuery,
) -> Result<DeliveryStats, Error> {
    // Read the time zone of the workspace, and find the buckets of the period in it.
    let mut tx = ctx.pool.begin().await?;
    let zone = deliveries::time_zone(&mut tx, query.workspace).await?;
    let starts = match query.bucket {
        Some(span) => query
            .period
            .starts(zone, span)
            .map_err(|error| Error::invalid("bucket", error.to_string()))?,
        None => Vec::new(),
    };
    let mut bounds = vec![query.period.from];
    bounds.extend(starts.iter().skip(1).copied());

    // Count the deliveries of the connections and the fires of the webhook triggers.
    let (workspace, period) = (query.workspace, query.period);
    let connections = deliveries::connections(&mut tx, workspace, period, &bounds).await?;
    let lasts = deliveries::lasts(&mut tx, workspace, period).await?;
    let visible = query.visible.as_deref();
    let triggers = deliveries::triggers(&mut tx, workspace, visible, period, &bounds).await?;
    tx.commit().await?;
    Ok(DeliveryStats::new(
        &starts,
        &connections,
        lasts,
        &triggers,
        zone,
    ))
}
