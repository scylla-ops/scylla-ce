//! The SQL of the `agent_samples` table: the samples of the use of the machines of the agents.
//! Each function runs in the transaction of the caller.

use std::collections::HashMap;

use scylla_domain::identity::agents::{HostSample, Sample, SampleSums};
use scylla_domain::identity::apps::AppId;
use scylla_domain::TimeZone;
use scylla_extension::Tx;
use time::{Duration, OffsetDateTime};

/// Inserts a sample of an agent at `at`, unless the agent has a sample less than `gap` before.
/// Returns `false` for a sample that it ignores.
pub(crate) async fn insert(
    tx: &mut Tx<'_>,
    agent: AppId,
    sample: Sample,
    at: OffsetDateTime,
    gap: Duration,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        "INSERT INTO agent_samples (agent_id, at, cpu_percent, memory_used_bytes)
         SELECT $1, $2, $3, $4
         WHERE NOT EXISTS (SELECT 1 FROM agent_samples WHERE agent_id = $1 AND at > $5)
         ON CONFLICT DO NOTHING",
        agent.to_db(),
        at,
        sample.cpu_percent,
        i64::try_from(sample.memory_used_bytes).unwrap_or(i64::MAX),
        at - gap,
    )
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Returns the last sample of each agent of `agents` with a sample.
pub(crate) async fn last(
    tx: &mut Tx<'_>,
    agents: &[AppId],
) -> Result<HashMap<AppId, HostSample>, sqlx::Error> {
    let ids: Vec<uuid::Uuid> = agents.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query!(
        "SELECT DISTINCT ON (agent_id) agent_id, at, cpu_percent, memory_used_bytes
         FROM agent_samples WHERE agent_id = ANY($1)
         ORDER BY agent_id, at DESC",
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let sample = HostSample {
                at: row.at,
                sample: Sample {
                    cpu_percent: row.cpu_percent,
                    memory_used_bytes: row.memory_used_bytes.max(0) as u64,
                },
            };
            (AppId::from_db(row.agent_id), sample)
        })
        .collect())
}

/// Returns the sums of the samples of an agent in each part of `bounds` with a sample, with its
/// number: number 1 counts the samples from the first bound to the second, and the last number
/// counts them to `end`.
pub(crate) async fn sums(
    tx: &mut Tx<'_>,
    agent: AppId,
    bounds: &[OffsetDateTime],
    end: OffsetDateTime,
) -> Result<Vec<(usize, SampleSums)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT width_bucket(at, $2::timestamptz[]) AS "part!",
                  count(*) AS "samples!",
                  sum(cpu_percent)::float8 AS "cpu_sum!",
                  max(cpu_percent) AS "cpu_max!",
                  sum(memory_used_bytes)::float8 AS "memory_sum!",
                  max(memory_used_bytes) AS "memory_max!"
           FROM agent_samples
           WHERE agent_id = $1 AND at >= ($2::timestamptz[])[1] AND at < $3
           GROUP BY 1"#,
        agent.to_db(),
        bounds,
        end,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let sums = SampleSums {
                samples: u32::try_from(row.samples).unwrap_or(u32::MAX),
                cpu_sum: row.cpu_sum,
                cpu_max: row.cpu_max,
                memory_sum: row.memory_sum,
                memory_max: row.memory_max.max(0) as u64,
            };
            Some((usize::try_from(row.part).ok()?, sums))
        })
        .collect())
}

/// Returns the time zone of the workspace of an agent, or UTC for an unknown agent.
pub(crate) async fn time_zone(tx: &mut Tx<'_>, agent: AppId) -> Result<TimeZone, sqlx::Error> {
    let name = sqlx::query_scalar!(
        "SELECT w.time_zone FROM apps a JOIN workspaces w ON w.id = a.workspace_id
         WHERE a.id = $1",
        agent.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(name
        .and_then(|name| TimeZone::parse(name).ok())
        .unwrap_or_default())
}

/// Deletes the samples before `before`. Returns their number.
pub(crate) async fn purge(tx: &mut Tx<'_>, before: OffsetDateTime) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM agent_samples WHERE at < $1", before)
        .execute(&mut **tx)
        .await?;
    Ok(result.rows_affected())
}
