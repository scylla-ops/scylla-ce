//! The SQL of the busy slots of the agents: each job that started holds one slot from its start to
//! its end. The parts come as times: the SQL knows no time zone.

use scylla_domain::identity::agents::Busy;
use scylla_domain::identity::apps::AppId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;
use time::{Duration, OffsetDateTime};

/// Returns the busy time of the jobs of a workspace, or of one agent, in each part of `bounds`
/// with a job, with its number: number 1 is the part from the first bound to the second, and the
/// last part ends at `end`. A job that runs holds its slot up to `end`.
pub(crate) async fn busy(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    agent: Option<AppId>,
    bounds: &[OffsetDateTime],
    end: OffsetDateTime,
) -> Result<Vec<(usize, Busy)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"WITH spans AS (
               SELECT greatest(started_at, ($4::timestamptz[])[1]) AS s,
                      least(coalesce(finished_at, $3), $3) AS e
               FROM jobs
               WHERE workspace_id = $1 AND ($2::uuid IS NULL OR agent_id = $2)
                 AND started_at IS NOT NULL AND started_at < $3
                 AND (finished_at IS NULL OR finished_at > ($4::timestamptz[])[1])
           ),
           steps AS (
               SELECT s AS at, 1 AS step FROM spans WHERE e > s
               UNION ALL SELECT e, -1 FROM spans WHERE e > s
               UNION ALL SELECT unnest($4::timestamptz[]), 0
           ),
           levels AS (
               SELECT at, (sum(sum(step)) OVER (ORDER BY at))::int8 AS busy
               FROM steps GROUP BY at
           ),
           segments AS (
               SELECT at, busy, lead(at, 1, $3::timestamptz) OVER (ORDER BY at) AS next
               FROM levels
           )
           SELECT width_bucket(at, $4::timestamptz[]) AS "part!",
                  sum(busy * extract(epoch FROM next - at))::float8 AS "seconds!",
                  max(busy) AS "max!"
           FROM segments
           WHERE at < $3
           GROUP BY 1"#,
        workspace.to_db(),
        agent.map(AppId::to_db),
        end,
        bounds,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let busy = Busy {
                time: Duration::checked_seconds_f64(row.seconds).unwrap_or_default(),
                max: u32::try_from(row.max).unwrap_or(0),
            };
            Some((usize::try_from(row.part).ok()?, busy))
        })
        .collect())
}
