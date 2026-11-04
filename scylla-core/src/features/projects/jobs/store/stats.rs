//! The SQL of the job stats of the agents.

use scylla_domain::identity::apps::AppId;
use scylla_domain::projects::jobs::{DayOutcomes, Outcomes};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;
use time::{Duration, OffsetDateTime};

/// The jobs of an agent since a time, by their end, and their durations.
pub(crate) struct Totals {
    /// The agent.
    pub(crate) agent_id: AppId,
    /// The jobs by their end.
    pub(crate) outcomes: Outcomes,
    /// The median time from the start to the end.
    pub(crate) median: Option<Duration>,
    /// The 95th percentile of the same times.
    pub(crate) p95: Option<Duration>,
}

/// Returns the totals of each agent of a workspace, or of `agent` only, for the jobs created
/// since `since`. An agent with no job is not in the list.
pub(crate) async fn totals(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    agent: Option<AppId>,
    since: OffsetDateTime,
) -> Result<Vec<Totals>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT agent_id AS "agent_id!",
                  count(*) FILTER (WHERE status = 'completed') AS "completed!",
                  count(*) FILTER (WHERE status = 'failed') AS "failed!",
                  count(*) FILTER (WHERE status = 'cancelled') AS "cancelled!",
                  count(*) FILTER (WHERE status = 'orphaned') AS "orphaned!",
                  percentile_cont(0.5) WITHIN GROUP
                      (ORDER BY extract(epoch FROM finished_at - started_at)::float8) AS median,
                  percentile_cont(0.95) WITHIN GROUP
                      (ORDER BY extract(epoch FROM finished_at - started_at)::float8) AS p95
           FROM jobs
           WHERE workspace_id = $1 AND agent_id IS NOT NULL
             AND ($2::uuid IS NULL OR agent_id = $2) AND created_at >= $3
           GROUP BY agent_id ORDER BY agent_id"#,
        workspace.to_db(),
        agent.map(AppId::to_db),
        since,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Totals {
            agent_id: AppId::from_db(row.agent_id),
            outcomes: outcomes(row.completed, row.failed, row.cancelled, row.orphaned),
            median: row.median.and_then(Duration::checked_seconds_f64),
            p95: row.p95.and_then(Duration::checked_seconds_f64),
        })
        .collect())
}

/// Returns the days with a job of each agent of a workspace, or of `agent` only, since
/// `since`, in the order of the agents and then of the days.
pub(crate) async fn days(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    agent: Option<AppId>,
    since: OffsetDateTime,
) -> Result<Vec<(AppId, DayOutcomes)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT agent_id AS "agent_id!", date_trunc('day', created_at, 'UTC') AS "day!",
                  count(*) FILTER (WHERE status = 'completed') AS "completed!",
                  count(*) FILTER (WHERE status = 'failed') AS "failed!",
                  count(*) FILTER (WHERE status = 'cancelled') AS "cancelled!",
                  count(*) FILTER (WHERE status = 'orphaned') AS "orphaned!"
           FROM jobs
           WHERE workspace_id = $1 AND agent_id IS NOT NULL
             AND ($2::uuid IS NULL OR agent_id = $2) AND created_at >= $3
           GROUP BY agent_id, 2 ORDER BY agent_id, 2"#,
        workspace.to_db(),
        agent.map(AppId::to_db),
        since,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let outcomes = outcomes(row.completed, row.failed, row.cancelled, row.orphaned);
            let day = DayOutcomes {
                day: row.day,
                outcomes,
            };
            (AppId::from_db(row.agent_id), day)
        })
        .collect())
}

/// Makes the outcomes of four counts.
fn outcomes(completed: i64, failed: i64, cancelled: i64, orphaned: i64) -> Outcomes {
    let count = |value: i64| u32::try_from(value).unwrap_or(u32::MAX);
    Outcomes {
        completed: count(completed),
        failed: count(failed),
        cancelled: count(cancelled),
        orphaned: count(orphaned),
    }
}
