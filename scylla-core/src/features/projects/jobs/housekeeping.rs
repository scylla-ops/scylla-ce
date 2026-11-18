//! The background task of the jobs: the jobs of the agents that went away, the old logs, the old
//! samples of the machines of the agents, and the old rows of the activity feed.

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use time::OffsetDateTime;
use tokio::task::JoinHandle;

use super::runs;
use super::store::{holds, logs};
use crate::features::activity;
use crate::features::identity::agents::{self, Registry};
use crate::Error;

/// The time between two rounds.
const TICK: Duration = Duration::from_secs(30);

/// The time after which an agent with no stream loses its jobs.
pub(crate) const GRACE: Duration = Duration::from_secs(60);

/// Starts the task: each 30 seconds, it releases the jobs of the agents that went away, deletes
/// the logs older than `retention`, the samples of the agents of more than 7 days and the rows
/// of the activity feed of more than 30 days.
pub(crate) fn spawn(pool: PgPool, registry: Arc<Registry>, retention: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + TICK, TICK);
        loop {
            ticks.tick().await;
            if let Err(error) = reap(&pool, &registry, GRACE).await {
                tracing::warn!(%error, "the jobs of the lost agents stay");
            }
            if let Err(error) = purge(&pool, retention).await {
                tracing::warn!(%error, "the old logs stay");
            }
            if let Err(error) = agents::purge_samples(&pool).await {
                tracing::warn!(%error, "the old samples of the agents stay");
            }
            if let Err(error) = activity::purge(&pool).await {
                tracing::warn!(%error, "the old rows of the activity stay");
            }
        }
    })
}

/// Releases the jobs whose agent has no stream and was last seen before `grace`: a running job
/// ends as orphaned, and a pending job goes back to the pool. Returns the number of jobs.
pub(crate) async fn reap(
    pool: &PgPool,
    registry: &Registry,
    grace: Duration,
) -> Result<usize, Error> {
    let before = OffsetDateTime::now_utc() - grace;
    let mut tx = pool.begin().await?;
    let stale = holds::held_by_stale(&mut tx, before).await?;
    let (mut count, mut pooled) = (0, false);
    for (id, agent) in stale {
        if registry.is_connected(agent) {
            continue;
        }
        pooled |= runs::release(&mut tx, id, "the agent went away").await?;
        count += 1;
    }
    tx.commit().await?;
    if pooled {
        registry.wake_all();
    }
    Ok(count)
}

/// Deletes the logs older than `retention`. Returns the number of batches.
pub(crate) async fn purge(pool: &PgPool, retention: Duration) -> Result<u64, Error> {
    let before = OffsetDateTime::now_utc() - retention;
    let mut tx = pool.begin().await?;
    let count = logs::purge(&mut tx, before).await?;
    tx.commit().await?;
    Ok(count)
}
