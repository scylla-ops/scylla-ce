//! The SQL of the job lists: the jobs of a pipeline, or the jobs of a workspace that a caller can
//! see, with the filters of the request.

use scylla_domain::identity::apps::AppId;
use scylla_domain::projects::jobs::{Job, JobFilter, JobId, JobScope};
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::ProjectId;
use scylla_domain::PageRequest;
use scylla_extension::Tx;
use serde_json::json;

use super::row::Row;
use super::sources;
use crate::Error;

/// The place of a list in the columns of `jobs`.
#[derive(Default)]
struct Place {
    /// The pipeline.
    pipeline: Option<uuid::Uuid>,
    /// The workspace.
    workspace: Option<uuid::Uuid>,
    /// The agent.
    agent: Option<uuid::Uuid>,
    /// `true` when the caller sees each project of the place.
    all: bool,
    /// The projects that the caller sees, when it does not see each project.
    visible: Vec<uuid::Uuid>,
    /// Only these projects, when the list is not empty.
    projects: Vec<uuid::Uuid>,
    /// Only these pipelines, when the list is not empty.
    pipelines: Vec<uuid::Uuid>,
}

impl Place {
    /// Returns the place of a scope.
    fn of(scope: &JobScope) -> Self {
        match scope {
            JobScope::Pipeline(id) => Self {
                pipeline: Some(id.to_db()),
                all: true,
                ..Self::default()
            },
            JobScope::Workspace(jobs) => Self {
                workspace: Some(jobs.workspace.to_db()),
                agent: jobs.agent.map(AppId::to_db),
                all: jobs.visible.is_none(),
                visible: jobs.visible.iter().flatten().map(|id| id.to_db()).collect(),
                projects: jobs
                    .projects
                    .iter()
                    .copied()
                    .map(ProjectId::to_db)
                    .collect(),
                pipelines: jobs
                    .pipelines
                    .iter()
                    .copied()
                    .map(PipelineId::to_db)
                    .collect(),
                ..Self::default()
            },
        }
    }
}

/// Returns the jobs of a scope before the page id that match the filter, newest first, with one
/// extra row.
pub(crate) async fn list(
    tx: &mut Tx<'_>,
    scope: &JobScope,
    page: PageRequest<JobId>,
    filter: &JobFilter,
) -> Result<Vec<Job>, Error> {
    // Read the rows of the page.
    let place = Place::of(scope);
    let pull_request = filter
        .pull_request
        .map(|number| json!([{ "number": number }]));
    let statuses: Vec<&str> = filter.statuses.iter().map(|one| one.as_str()).collect();
    let origins: Vec<&str> = filter.origins.iter().map(|one| one.as_str()).collect();
    let rows = sqlx::query_as!(
        Row,
        "SELECT j.id, j.pipeline_id, j.project_id, j.workspace_id, j.created_by, j.rerun_of,
                j.trigger_id, j.origin, j.status, j.nodes, j.runs, j.agent_id, j.error, j.version,
                j.created_at, j.started_at, j.finished_at, j.approval, j.approved_by,
                j.approved_at, j.rejected_by, j.rejected_at, j.checkout, j.pipeline_file,
                j.file_errors, j.agent_labels
         FROM jobs j LEFT JOIN job_sources s ON s.job_id = j.id
         WHERE ($1::uuid IS NULL OR j.pipeline_id = $1)
           AND ($2::uuid IS NULL OR j.workspace_id = $2)
           AND ($3::uuid IS NULL OR j.agent_id = $3)
           AND ($4::uuid IS NULL OR j.id < $4)
           AND ($6::text IS NULL OR s.branch = $6)
           AND ($7::text IS NULL OR s.event = $7)
           AND ($8::text IS NULL OR s.author_login = $8)
           AND ($9::jsonb IS NULL OR s.pull_requests @> $9)
           AND ($10::bool OR j.project_id = ANY($11::uuid[]))
           AND (cardinality($12::uuid[]) = 0 OR j.project_id = ANY($12))
           AND (cardinality($13::uuid[]) = 0 OR j.pipeline_id = ANY($13))
           AND (cardinality($14::text[]) = 0 OR j.status = ANY($14))
           AND (cardinality($15::text[]) = 0 OR j.origin = ANY($15))
           AND ($16::timestamptz IS NULL OR j.created_at >= $16)
           AND ($17::timestamptz IS NULL OR j.created_at < $17)
           AND ($18 = ''
                OR strpos(right(j.id::text, 8), $18) > 0
                OR strpos(lower(coalesce(s.branch, '')), $18) > 0
                OR strpos(lower(coalesce(s.tag, '')), $18) > 0
                OR strpos(lower(coalesce(s.message_title, '')), $18) > 0
                OR EXISTS (SELECT 1 FROM pipelines p
                           WHERE p.id = j.pipeline_id AND strpos(lower(p.name), $18) > 0))
         ORDER BY j.id DESC
         LIMIT $5",
        place.pipeline,
        place.workspace,
        place.agent,
        page.after.map(JobId::to_db),
        page.size.get() as i64 + 1,
        filter.branch,
        filter.event.map(|event| event.as_str()),
        filter.author_login,
        pull_request,
        place.all,
        &place.visible,
        &place.projects,
        &place.pipelines,
        &statuses as &[&str],
        &origins as &[&str],
        filter.created_after,
        filter.created_before,
        filter.query.as_str(),
    )
    .fetch_all(&mut **tx)
    .await?;

    // Add the source of each job.
    let ids: Vec<JobId> = rows.iter().map(|row| JobId::from_db(row.id)).collect();
    let mut found = sources::of_jobs(tx, &ids).await?;
    rows.into_iter()
        .map(|row| {
            let source = found.remove(&JobId::from_db(row.id));
            row.into_job(source)
        })
        .collect()
}
