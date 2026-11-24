//! The SQL of the figures of the jobs of a workspace: the counts and the times of the jobs of a
//! period, for all the jobs, for each bucket or for each group. The buckets come as times: the
//! SQL knows no time zone.

use scylla_domain::identity::apps::AppId;
use scylla_domain::projects::jobs::{
    Figures, FiguresPeriod, GroupBy, GroupKey, JobOrigin, Outcomes, WorkspaceJobs,
};
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::TimeZone;
use scylla_extension::Tx;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

/// The jobs to count: the workspace, the projects that the caller can see, the filters and the
/// period.
#[derive(Clone, Copy)]
pub(crate) struct Slice<'a> {
    /// The workspace, the projects that the caller can see, and the filters of the request.
    pub(crate) jobs: &'a WorkspaceJobs,
    /// Only the jobs with one of these origins. Empty gives each origin.
    pub(crate) origins: &'a [JobOrigin],
    /// The period of the creation of the jobs.
    pub(crate) period: FiguresPeriod,
}

/// How a query splits the jobs.
#[derive(Clone, Copy)]
enum Split<'a> {
    /// No split: one row for all the jobs.
    Whole,
    /// One row for each bucket of these bounds: the first bound is the start of the period.
    Buckets(&'a [OffsetDateTime]),
    /// One row for each group.
    Groups(GroupBy),
}

/// The figures of one row, with its key.
struct Row {
    /// The project, the pipeline or the agent of a group.
    id: Option<Uuid>,
    /// The origin of a group.
    origin: Option<String>,
    /// The number of a bucket, from 1.
    bucket: Option<i32>,
    /// The figures.
    figures: Figures,
}

/// Returns the time zone of a workspace, or UTC for an unknown workspace.
pub(crate) async fn time_zone(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
) -> Result<TimeZone, sqlx::Error> {
    let name = sqlx::query_scalar!(
        "SELECT time_zone FROM workspaces WHERE id = $1",
        workspace.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(name
        .and_then(|name| TimeZone::parse(name).ok())
        .unwrap_or_default())
}

/// Returns the figures of all the jobs of a slice.
pub(crate) async fn totals(tx: &mut Tx<'_>, slice: Slice<'_>) -> Result<Figures, sqlx::Error> {
    let rows = rows(tx, slice, Split::Whole).await?;
    Ok(rows.first().map(|row| row.figures).unwrap_or_default())
}

/// Returns the figures of each bucket with a job, with its number: number 1 counts the jobs from
/// the start of the period to the second start, and the last number counts them to the end of the
/// period.
pub(crate) async fn buckets(
    tx: &mut Tx<'_>,
    slice: Slice<'_>,
    starts: &[OffsetDateTime],
) -> Result<Vec<(usize, Figures)>, sqlx::Error> {
    let mut bounds = vec![slice.period.from];
    bounds.extend(starts.iter().skip(1).copied());
    let rows = rows(tx, slice, Split::Buckets(&bounds)).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| Some((usize::try_from(row.bucket?).ok()?, row.figures)))
        .collect())
}

/// Returns the figures of each group with a job. A job with no agent is in no group of agents.
pub(crate) async fn groups(
    tx: &mut Tx<'_>,
    slice: Slice<'_>,
    by: GroupBy,
) -> Result<Vec<(GroupKey, Figures)>, sqlx::Error> {
    let rows = rows(tx, slice, Split::Groups(by)).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| Some((key(by, &row)?, row.figures)))
        .collect())
}

/// Returns the key of the group of a row, or `None` for a row with no key.
fn key(by: GroupBy, row: &Row) -> Option<GroupKey> {
    match by {
        GroupBy::Project => row.id.map(|id| GroupKey::Project(ProjectId::from_db(id))),
        GroupBy::Pipeline => row.id.map(|id| GroupKey::Pipeline(PipelineId::from_db(id))),
        GroupBy::Agent => row.id.map(|id| GroupKey::Agent(AppId::from_db(id))),
        GroupBy::Origin => row
            .origin
            .as_deref()
            .and_then(JobOrigin::parse)
            .map(GroupKey::Origin),
    }
}

/// Counts the jobs of a slice with a split, and returns one row for each part with a job.
async fn rows(
    tx: &mut Tx<'_>,
    slice: Slice<'_>,
    split: Split<'_>,
) -> Result<Vec<Row>, sqlx::Error> {
    // Name the split, and read the filters of the slice.
    let (name, bounds): (&str, &[OffsetDateTime]) = match split {
        Split::Whole => ("whole", &[]),
        Split::Buckets(bounds) => ("bucket", bounds),
        Split::Groups(GroupBy::Project) => ("project", &[]),
        Split::Groups(GroupBy::Pipeline) => ("pipeline", &[]),
        Split::Groups(GroupBy::Agent) => ("agent", &[]),
        Split::Groups(GroupBy::Origin) => ("origin", &[]),
    };
    let jobs = slice.jobs;
    let visible: Vec<Uuid> = jobs.visible.iter().flatten().map(|id| id.to_db()).collect();
    let projects: Vec<Uuid> = jobs.projects.iter().map(|id| id.to_db()).collect();
    let pipelines: Vec<Uuid> = jobs.pipelines.iter().map(|id| id.to_db()).collect();
    let origins: Vec<&str> = slice.origins.iter().map(|one| one.as_str()).collect();

    // Count the jobs of each part.
    let rows = sqlx::query!(
        r#"SELECT CASE $1::text WHEN 'project' THEN project_id WHEN 'pipeline' THEN pipeline_id
                                WHEN 'agent' THEN agent_id END AS id,
                  CASE $1::text WHEN 'origin' THEN origin END AS origin,
                  CASE $1::text WHEN 'bucket' THEN width_bucket(created_at, $2::timestamptz[])
                  END AS bucket,
                  count(*) AS "created!",
                  count(*) FILTER (WHERE status = 'completed') AS "completed!",
                  count(*) FILTER (WHERE status = 'failed') AS "failed!",
                  count(*) FILTER (WHERE status = 'cancelled') AS "cancelled!",
                  count(*) FILTER (WHERE status = 'orphaned') AS "orphaned!",
                  percentile_cont(0.5) WITHIN GROUP
                      (ORDER BY extract(epoch FROM finished_at - started_at)::float8) AS p50,
                  percentile_cont(0.95) WITHIN GROUP
                      (ORDER BY extract(epoch FROM finished_at - started_at)::float8) AS p95,
                  percentile_cont(0.5) WITHIN GROUP (ORDER BY extract(epoch FROM
                      started_at - coalesce(approved_at, created_at))::float8) AS p50_queue,
                  percentile_cont(0.95) WITHIN GROUP (ORDER BY extract(epoch FROM
                      started_at - coalesce(approved_at, created_at))::float8) AS p95_queue,
                  coalesce(sum(extract(epoch FROM finished_at - started_at)::float8), 0)
                      AS "run_time!"
           FROM jobs
           WHERE workspace_id = $3
             AND ($4::bool OR project_id = ANY($5::uuid[]))
             AND (cardinality($6::uuid[]) = 0 OR project_id = ANY($6))
             AND (cardinality($7::uuid[]) = 0 OR pipeline_id = ANY($7))
             AND ($8::uuid IS NULL OR agent_id = $8)
             AND (cardinality($9::text[]) = 0 OR origin = ANY($9))
             AND created_at >= $10 AND created_at < $11
           GROUP BY 1, 2, 3"#,
        name,
        bounds,
        jobs.workspace.to_db(),
        jobs.visible.is_none(),
        &visible,
        &projects,
        &pipelines,
        jobs.agent.map(AppId::to_db),
        &origins as &[&str],
        slice.period.from,
        slice.period.to,
    )
    .fetch_all(&mut **tx)
    .await?;

    // Make the figures of each row.
    let time = |seconds: Option<f64>| seconds.and_then(Duration::checked_seconds_f64);
    Ok(rows
        .into_iter()
        .map(|row| Row {
            id: row.id,
            origin: row.origin,
            bucket: row.bucket,
            figures: Figures {
                created: count(row.created),
                outcomes: Outcomes {
                    completed: count(row.completed),
                    failed: count(row.failed),
                    cancelled: count(row.cancelled),
                    orphaned: count(row.orphaned),
                },
                p50_duration: time(row.p50),
                p95_duration: time(row.p95),
                p50_queue: time(row.p50_queue),
                p95_queue: time(row.p95_queue),
                run_time: time(Some(row.run_time)).unwrap_or_default(),
            },
        })
        .collect())
}

/// Returns a count of the database as a number of jobs.
fn count(value: i64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
