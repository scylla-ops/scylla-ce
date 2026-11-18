//! The SQL of the counts of the fires of a workspace: for all the fires of a period, for each
//! bucket or for each group. The buckets come as times: the SQL knows no time zone.

use scylla_domain::projects::jobs::FiguresPeriod;
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::triggers::{
    FireFigures, FireGroup, FireGroupBy, FireGroupKey, TriggerId, WorkspaceFires,
};
use scylla_domain::projects::ProjectId;
use scylla_extension::Tx;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

/// How a query splits the fires.
#[derive(Clone, Copy)]
enum Split<'a> {
    /// No split: one row for all the fires.
    Whole,
    /// One row for each bucket of these bounds: the first bound is the start of the period.
    Buckets(&'a [OffsetDateTime]),
    /// One row for each group.
    Groups(FireGroupBy),
}

/// The counts of one row, with its key.
struct Row {
    /// The trigger, the pipeline or the project of a group.
    id: Option<Uuid>,
    /// The number of a bucket, from 1.
    bucket: Option<i32>,
    /// The counts.
    figures: FireFigures,
}

/// Returns the counts of all the fires of a period.
pub(crate) async fn totals(
    tx: &mut Tx<'_>,
    fires: &WorkspaceFires,
    period: FiguresPeriod,
) -> Result<FireFigures, sqlx::Error> {
    let rows = rows(tx, fires, period, Split::Whole).await?;
    Ok(rows.first().map(|row| row.figures).unwrap_or_default())
}

/// Returns the counts of each bucket with a fire, with its number: number 1 counts the fires from
/// the start of the period to the second start, and the last number counts them to the end of
/// the period.
pub(crate) async fn buckets(
    tx: &mut Tx<'_>,
    fires: &WorkspaceFires,
    period: FiguresPeriod,
    starts: &[OffsetDateTime],
) -> Result<Vec<(usize, FireFigures)>, sqlx::Error> {
    let mut bounds = vec![period.from];
    bounds.extend(starts.iter().skip(1).copied());
    let rows = rows(tx, fires, period, Split::Buckets(&bounds)).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| Some((usize::try_from(row.bucket?).ok()?, row.figures)))
        .collect())
}

/// Returns the counts of each group with a fire.
pub(crate) async fn groups(
    tx: &mut Tx<'_>,
    fires: &WorkspaceFires,
    period: FiguresPeriod,
    by: FireGroupBy,
) -> Result<Vec<FireGroup>, sqlx::Error> {
    let rows = rows(tx, fires, period, Split::Groups(by)).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let id = row.id?;
            let key = match by {
                FireGroupBy::Trigger => FireGroupKey::Trigger(TriggerId::from_db(id)),
                FireGroupBy::Pipeline => FireGroupKey::Pipeline(PipelineId::from_db(id)),
                FireGroupBy::Project => FireGroupKey::Project(ProjectId::from_db(id)),
            };
            Some(FireGroup {
                key,
                figures: row.figures,
            })
        })
        .collect())
}

/// Counts the fires of a period with a split, and returns one row for each part with a fire.
async fn rows(
    tx: &mut Tx<'_>,
    fires: &WorkspaceFires,
    period: FiguresPeriod,
    split: Split<'_>,
) -> Result<Vec<Row>, sqlx::Error> {
    // Name the split, and read the filters.
    let (name, bounds): (&str, &[OffsetDateTime]) = match split {
        Split::Whole => ("whole", &[]),
        Split::Buckets(bounds) => ("bucket", bounds),
        Split::Groups(FireGroupBy::Trigger) => ("trigger", &[]),
        Split::Groups(FireGroupBy::Pipeline) => ("pipeline", &[]),
        Split::Groups(FireGroupBy::Project) => ("project", &[]),
    };
    let visible: Vec<Uuid> = fires
        .visible
        .iter()
        .flatten()
        .map(|id| id.to_db())
        .collect();
    let projects: Vec<Uuid> = fires.projects.iter().map(|id| id.to_db()).collect();
    let pipelines: Vec<Uuid> = fires.pipelines.iter().map(|id| id.to_db()).collect();
    let triggers: Vec<Uuid> = fires.triggers.iter().map(|id| id.to_db()).collect();
    let sources: Vec<&str> = fires.sources.iter().map(|one| one.as_str()).collect();

    // Count the fires of each part.
    let rows = sqlx::query!(
        r#"SELECT CASE $1::text WHEN 'trigger' THEN trigger_id WHEN 'pipeline' THEN pipeline_id
                                WHEN 'project' THEN project_id END AS id,
                  CASE $1::text WHEN 'bucket' THEN width_bucket(due_at, $2::timestamptz[])
                  END AS bucket,
                  count(*) AS "total!",
                  count(*) FILTER (WHERE status = 'ok') AS "ok!",
                  count(*) FILTER (WHERE status = 'error') AS "failed!",
                  count(*) FILTER (WHERE status = 'missed') AS "missed!",
                  percentile_cont(0.5) WITHIN GROUP
                      (ORDER BY extract(epoch FROM fired_at - due_at)::float8)
                      FILTER (WHERE source = 'schedule' AND fired_at IS NOT NULL) AS p50,
                  percentile_cont(0.95) WITHIN GROUP
                      (ORDER BY extract(epoch FROM fired_at - due_at)::float8)
                      FILTER (WHERE source = 'schedule' AND fired_at IS NOT NULL) AS p95
           FROM trigger_fires
           WHERE workspace_id = $3
             AND ($4::bool OR project_id = ANY($5::uuid[]))
             AND (cardinality($6::uuid[]) = 0 OR project_id = ANY($6))
             AND (cardinality($7::uuid[]) = 0 OR pipeline_id = ANY($7))
             AND (cardinality($8::uuid[]) = 0 OR trigger_id = ANY($8))
             AND (cardinality($9::text[]) = 0 OR source = ANY($9))
             AND due_at >= $10 AND due_at < $11
           GROUP BY 1, 2"#,
        name,
        bounds,
        fires.workspace.to_db(),
        fires.visible.is_none(),
        &visible,
        &projects,
        &pipelines,
        &triggers,
        &sources as &[&str],
        period.from,
        period.to,
    )
    .fetch_all(&mut **tx)
    .await?;

    // Make the counts of each row.
    let time = |seconds: Option<f64>| seconds.and_then(Duration::checked_seconds_f64);
    let count = |value: i64| u32::try_from(value).unwrap_or(u32::MAX);
    Ok(rows
        .into_iter()
        .map(|row| Row {
            id: row.id,
            bucket: row.bucket,
            figures: FireFigures {
                total: count(row.total),
                ok: count(row.ok),
                failed: count(row.failed),
                missed: count(row.missed),
                p50_delay: time(row.p50),
                p95_delay: time(row.p95),
            },
        })
        .collect())
}
