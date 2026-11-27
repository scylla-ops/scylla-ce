//! The SQL of the summaries of the pipelines: the figures of the recent jobs, the health and the
//! triggers of each pipeline, and the recent jobs of a page.

use std::collections::HashMap;

use scylla_domain::integrations::links::PipelineSource;
use scylla_domain::projects::jobs::{JobId, JobStatus};
use scylla_domain::projects::pipelines::{
    ListSummaries, NextFire, PipelineId, PipelineSummary, RecentJob, TriggerType,
};
use scylla_domain::projects::triggers::TriggerId;
use scylla_domain::projects::{ProjectId, Visible};
use scylla_extension::Tx;
use time::Duration;

use crate::Error;

/// Returns the summary of each pipeline of a workspace that the caller can see and that the
/// filters keep, with no recent job: the figures of the recent jobs, the last ended job of the
/// default branch, the state of the jobs that run or wait, and the triggers that can start a
/// job. In a project of files, a Git trigger of an editor pipeline can start a job only with the
/// opt-in.
pub(crate) async fn summaries(
    tx: &mut Tx<'_>,
    query: &ListSummaries,
    visible: &Visible,
) -> Result<Vec<PipelineSummary>, Error> {
    // Read one row for each pipeline.
    let projects: Vec<_> = query.project_ids.iter().map(|id| id.to_db()).collect();
    let pipelines: Vec<_> = query.pipeline_ids.iter().map(|id| id.to_db()).collect();
    let workspaces: Vec<_> = visible.workspaces.iter().map(|id| id.to_db()).collect();
    let shared: Vec<_> = visible.projects.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query!(
        r#"WITH live AS (
               SELECT pipeline_id,
                      CASE WHEN bool_or(status = 'running') THEN 'running'
                           WHEN bool_or(status = 'pending') THEN 'pending'
                           ELSE 'awaiting_approval' END AS status
               FROM jobs
               WHERE workspace_id = $1 AND status IN ('awaiting_approval', 'pending', 'running')
               GROUP BY pipeline_id
           )
           SELECT p.id, p.name, p.project_id, r.name AS project_name,
                  last.status AS "last_ended?",
                  live.status AS "live?",
                  recent.completed AS "completed!", recent.failed AS "failed!", recent.median,
                  coalesce(kinds.names, '{}') AS "kinds!: Vec<String>",
                  fire.id AS "fire_id?", fire.schedule AS "fire_schedule?",
                  fire.next_fire_at AS "fire_at?", p.source, p.key_taken_at IS NOT NULL AS "taken!"
           FROM pipelines p
           JOIN projects r ON r.id = p.project_id
           LEFT JOIN repository_links l ON l.project_id = p.project_id
           LEFT JOIN live ON live.pipeline_id = p.id
           LEFT JOIN LATERAL (
               SELECT j.status FROM jobs j
               LEFT JOIN job_sources s ON s.job_id = j.id
               WHERE j.pipeline_id = p.id AND j.finished_at IS NOT NULL
                 AND (l.project_id IS NULL OR s.branch = l.default_branch)
               ORDER BY j.id DESC LIMIT 1
           ) last ON true
           CROSS JOIN LATERAL (
               SELECT count(*) FILTER (WHERE t.status = 'completed') AS completed,
                      count(*) FILTER (WHERE t.status IN ('failed', 'orphaned')) AS failed,
                      percentile_cont(0.5) WITHIN GROUP
                          (ORDER BY extract(epoch FROM t.finished_at - t.started_at)::float8)
                          FILTER (WHERE t.status = 'completed' AND t.started_at IS NOT NULL)
                          AS median
               FROM (SELECT status, started_at, finished_at FROM jobs
                     WHERE pipeline_id = p.id ORDER BY id DESC LIMIT $2) t
           ) recent
           LEFT JOIN LATERAL (
               SELECT array_agg(DISTINCT k.kind) AS names FROM (
                   SELECT t.kind FROM triggers t WHERE t.pipeline_id = p.id AND t.enabled
                     AND (t.kind NOT IN ('push', 'pull_request', 'tag')
                          OR l.pipeline_source IS DISTINCT FROM 'file'
                          OR p.git_events_allowed_at IS NOT NULL)
                   UNION ALL
                   SELECT f.kind FROM unnest(ARRAY['push', 'pull_request', 'tag']) AS f (kind)
                   WHERE p.file_rules ? f.kind
               ) k
           ) kinds ON p.archived_at IS NULL AND p.removed_at IS NULL
           LEFT JOIN LATERAL (
               SELECT t.id, t.schedule, t.next_fire_at FROM triggers t
               WHERE t.pipeline_id = p.id AND t.enabled AND t.kind = 'cron'
                 AND t.next_fire_at IS NOT NULL
               ORDER BY t.next_fire_at, t.id LIMIT 1
           ) fire ON p.archived_at IS NULL AND p.removed_at IS NULL
           WHERE p.workspace_id = $1
             AND (p.source = 'editor' OR p.archived_at IS NULL)
             AND (cardinality($3::uuid[]) = 0 OR p.project_id = ANY($3))
             AND (cardinality($4::uuid[]) = 0 OR p.id = ANY($4))
             AND ($5
                  OR p.workspace_id IN (SELECT o.workspace_id FROM workspace_owners o
                                        WHERE o.user_id = $6)
                  OR p.workspace_id = ANY($7)
                  OR p.project_id = ANY($8))"#,
        query.workspace_id.to_db(),
        i64::from(query.runs),
        &projects,
        &pipelines,
        visible.all,
        visible.caller.to_db(),
        &workspaces,
        &shared,
    )
    .fetch_all(&mut **tx)
    .await?;

    // Make the summaries.
    let count = |value: i64| u32::try_from(value).unwrap_or(u32::MAX);
    rows.into_iter()
        .map(|row| {
            let next_fire = match (row.fire_id, row.fire_schedule, row.fire_at) {
                (Some(id), Some(schedule), Some(at)) => Some(NextFire {
                    trigger_id: TriggerId::from_db(id),
                    schedule,
                    at,
                }),
                _ => None,
            };
            let mut trigger_kinds: Vec<_> = row
                .kinds
                .iter()
                .filter_map(|name| TriggerType::parse(name))
                .collect();
            trigger_kinds.sort();
            Ok(PipelineSummary {
                pipeline_id: PipelineId::from_db(row.id),
                name: row.name,
                project_id: ProjectId::from_db(row.project_id),
                project_name: row.project_name,
                recent: Vec::new(),
                completed: count(row.completed),
                failed: count(row.failed),
                median: row.median.and_then(Duration::checked_seconds_f64),
                last_ended: row.last_ended.as_deref().map(status).transpose()?,
                live: row.live.as_deref().map(status).transpose()?,
                trigger_kinds,
                next_fire,
                source: PipelineSource::parse(&row.source)
                    .ok_or_else(|| Error::Internal(format!("unknown source {}", row.source)))?,
                key_taken: row.taken,
            })
        })
        .collect()
}

/// Returns the last jobs of some pipelines, at most `runs` for each one, newest first.
pub(crate) async fn recent_jobs(
    tx: &mut Tx<'_>,
    pipelines: &[PipelineId],
    runs: u32,
) -> Result<HashMap<PipelineId, Vec<RecentJob>>, Error> {
    // Read the jobs.
    let ids: Vec<_> = pipelines.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query!(
        r#"SELECT p.id AS "pipeline_id!", j.id AS "id!", j.status AS "status!",
                  s.branch AS "branch?", j.created_at AS "created_at!", j.started_at,
                  j.finished_at
           FROM unnest($1::uuid[]) AS p (id)
           CROSS JOIN LATERAL (
               SELECT id, status, created_at, started_at, finished_at FROM jobs
               WHERE pipeline_id = p.id ORDER BY id DESC LIMIT $2
           ) j
           LEFT JOIN job_sources s ON s.job_id = j.id
           ORDER BY p.id, j.id DESC"#,
        &ids,
        i64::from(runs),
    )
    .fetch_all(&mut **tx)
    .await?;

    // Group the jobs by pipeline, in their order.
    let mut recent: HashMap<PipelineId, Vec<RecentJob>> = HashMap::new();
    for row in rows {
        let job = RecentJob {
            id: JobId::from_db(row.id),
            status: status(&row.status)?,
            branch: row.branch,
            created_at: row.created_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
        };
        recent
            .entry(PipelineId::from_db(row.pipeline_id))
            .or_default()
            .push(job);
    }
    Ok(recent)
}

/// Reads the state of a job, as the database keeps it.
fn status(name: &str) -> Result<JobStatus, Error> {
    JobStatus::parse(name).ok_or_else(|| Error::Internal(format!("unknown job state {name}")))
}
