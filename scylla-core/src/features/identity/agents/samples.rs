//! The samples of the use of the machines of the agents: one transaction for each call. The
//! stream records a sample, the Apps read the last ones, and `GetAgentMetrics` reads them in
//! buckets. The background task of the jobs deletes the samples of more than 7 days.

use std::collections::HashMap;

use scylla_domain::identity::agents::{
    self, AgentMetrics, HostSample, MetricsQuery, Sample, SAMPLE_GAP, SAMPLE_RETENTION,
};
use scylla_domain::identity::apps::AppId;
use sqlx::PgPool;
use time::OffsetDateTime;

use super::store;
use crate::Error;

/// Records a sample of an agent at the time of the server, unless the agent sent one less than
/// `SAMPLE_GAP` before. Returns `false` for a sample that it ignores.
pub(crate) async fn record(pool: &PgPool, agent: AppId, sample: Sample) -> Result<bool, Error> {
    let mut tx = pool.begin().await?;
    let now = OffsetDateTime::now_utc();
    let kept = store::insert(&mut tx, agent, sample, now, SAMPLE_GAP).await?;
    tx.commit().await?;
    Ok(kept)
}

/// Returns the last sample of each agent of `agents` with a sample.
pub(crate) async fn last(
    pool: &PgPool,
    agents: &[AppId],
) -> Result<HashMap<AppId, HostSample>, Error> {
    let mut tx = pool.begin().await?;
    let samples = store::last(&mut tx, agents).await?;
    tx.commit().await?;
    Ok(samples)
}

/// Returns the use of the machine of an agent in a period and in its buckets, in the time zone of
/// its workspace. Refuses more buckets than the most.
pub(crate) async fn metrics(pool: &PgPool, query: MetricsQuery) -> Result<AgentMetrics, Error> {
    // Read the time zone of the workspace, and find the buckets of the period in it.
    let mut tx = pool.begin().await?;
    let zone = store::time_zone(&mut tx, query.agent).await?;
    let starts = match query.bucket {
        Some(span) => query
            .period
            .starts(zone, span)
            .map_err(|error| Error::invalid("bucket", error.to_string()))?,
        None => Vec::new(),
    };

    // Add the samples of each part of the period.
    let bounds = agents::bounds(query.period, &starts);
    let found = store::sums(&mut tx, query.agent, &bounds, query.period.to).await?;
    tx.commit().await?;
    Ok(AgentMetrics::of_parts(&starts, &found, zone))
}

/// Deletes the samples of more than `SAMPLE_RETENTION`. Returns their number.
pub(crate) async fn purge(pool: &PgPool) -> Result<u64, Error> {
    let before = OffsetDateTime::now_utc() - SAMPLE_RETENTION;
    let mut tx = pool.begin().await?;
    let count = store::purge(&mut tx, before).await?;
    tx.commit().await?;
    Ok(count)
}
