//! The SQL of the `jobs` table. A job comes back with its source from `job_sources`.

use scylla_domain::identity::apps::AppId;
use scylla_domain::projects::jobs::{
    Job, JobFilter, JobId, JobScope, JobStatus, NewJob, NodeRun, INVALID_FILE,
};
use scylla_domain::projects::triggers::TriggerId;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::PageRequest;
use scylla_extension::Tx;
use serde_json::json;

use super::row::{self, bad_status, Row};
use super::sources;
use crate::features::projects::pipelines::json;
use crate::Error;

/// Inserts a job with its source, and returns it. A job of an invalid pipeline file fails at
/// once, and a job from a fork waits for an approval.
pub(crate) async fn insert(tx: &mut Tx<'_>, new: &NewJob) -> Result<Job, Error> {
    // Insert the row in its first state.
    let id = JobId::generate();
    let runs: Vec<NodeRun> = new.nodes.iter().map(NodeRun::pending).collect();
    let need_container = new.nodes.iter().any(|node| {
        !matches!(
            node.runtime,
            scylla_domain::projects::pipelines::Runtime::Process
        )
    });
    let status = new.status();
    let error = (status == JobStatus::Failed).then_some(INVALID_FILE);
    let approval = (status == JobStatus::AwaitingApproval).then_some("waiting");
    sqlx::query!(
        "INSERT INTO jobs (id, pipeline_id, project_id, workspace_id, created_by, rerun_of,
                           trigger_id, status, nodes, runs, need_container, approval, error,
                           pipeline_file, file_errors, finished_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
                 CASE WHEN $8 = 'failed' THEN now() END)",
        id.to_db(),
        new.pipeline_id.to_db(),
        new.project_id.to_db(),
        new.workspace_id.to_db(),
        new.created_by.to_db(),
        new.rerun_of.map(JobId::to_db),
        new.trigger_id.map(TriggerId::to_db),
        status.as_str(),
        json::nodes(&new.nodes),
        json::runs(&runs),
        need_container,
        approval,
        error,
        row::pipeline_file(new.pipeline_file.as_ref()),
        row::file_errors(&new.file_errors),
    )
    .execute(&mut **tx)
    .await?;

    // Insert its source, then read the job back.
    if let Some(source) = &new.source {
        sources::insert(tx, id, new.pipeline_id, source).await?;
    }
    find(tx, id, false)
        .await?
        .ok_or_else(|| Error::Internal(format!("the new job {id} is missing")))
}

/// Returns a job, or `None` if no job has this id. With `lock`, the row stays locked until the
/// end of the transaction.
pub(crate) async fn find(tx: &mut Tx<'_>, id: JobId, lock: bool) -> Result<Option<Job>, Error> {
    // Lock the row first, if the caller changes it.
    if lock {
        let locked =
            sqlx::query_scalar!("SELECT id FROM jobs WHERE id = $1 FOR UPDATE", id.to_db())
                .fetch_optional(&mut **tx)
                .await?;
        if locked.is_none() {
            return Ok(None);
        }
    }

    // Read the row and its source.
    let row = sqlx::query_as!(
        Row,
        "SELECT id, pipeline_id, project_id, workspace_id, created_by, rerun_of, trigger_id, status,
                nodes, runs, agent_id, error, version, created_at, started_at, finished_at,
                approval, approved_by, approved_at, rejected_by, rejected_at, checkout,
                pipeline_file, file_errors
         FROM jobs WHERE id = $1",
        id.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let source = sources::find(tx, id).await?;
    row.into_job(source).map(Some)
}

/// Returns the project and the workspace of a job, or `None` if no job has this id.
pub(crate) async fn place(
    tx: &mut Tx<'_>,
    id: JobId,
) -> Result<Option<(ProjectId, WorkspaceId)>, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT project_id, workspace_id FROM jobs WHERE id = $1",
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| {
        (
            ProjectId::from_db(row.project_id),
            WorkspaceId::from_db(row.workspace_id),
        )
    }))
}

/// Returns the jobs of a pipeline or of a workspace before the page id that match the filter,
/// newest first, with one extra row.
pub(crate) async fn list(
    tx: &mut Tx<'_>,
    scope: JobScope,
    page: PageRequest<JobId>,
    filter: &JobFilter,
) -> Result<Vec<Job>, Error> {
    // Read the rows of the page.
    let (pipeline, workspace, agent) = match scope {
        JobScope::Pipeline(id) => (Some(id.to_db()), None, None),
        JobScope::Workspace(id, agent) => (None, Some(id.to_db()), agent.map(AppId::to_db)),
    };
    let pull_request = filter
        .pull_request
        .map(|number| json!([{ "number": number }]));
    let rows = sqlx::query_as!(
        Row,
        "SELECT j.id, j.pipeline_id, j.project_id, j.workspace_id, j.created_by, j.rerun_of,
                j.trigger_id, j.status, j.nodes, j.runs, j.agent_id, j.error, j.version,
                j.created_at, j.started_at, j.finished_at, j.approval, j.approved_by,
                j.approved_at, j.rejected_by, j.rejected_at, j.checkout, j.pipeline_file,
                j.file_errors
         FROM jobs j LEFT JOIN job_sources s ON s.job_id = j.id
         WHERE ($1::uuid IS NULL OR j.pipeline_id = $1)
           AND ($2::uuid IS NULL OR j.workspace_id = $2)
           AND ($3::uuid IS NULL OR j.agent_id = $3)
           AND ($4::uuid IS NULL OR j.id < $4)
           AND ($6::text IS NULL OR s.branch = $6)
           AND ($7::text IS NULL OR s.event = $7)
           AND ($8::text IS NULL OR s.author_login = $8)
           AND ($9::jsonb IS NULL OR s.pull_requests @> $9)
         ORDER BY j.id DESC
         LIMIT $5",
        pipeline,
        workspace,
        agent,
        page.after.map(JobId::to_db),
        page.size.get() as i64 + 1,
        filter.branch,
        filter.event.map(|event| event.as_str()),
        filter.author_login,
        pull_request,
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

/// Writes the state of a job that the caller read in this transaction, with one more version.
pub(crate) async fn save(tx: &mut Tx<'_>, job: &mut Job) -> Result<(), Error> {
    let (approval, approved_by, approved_at, rejected_by, rejected_at) =
        row::approval_columns(job.approval.as_ref());
    let version = sqlx::query_scalar!(
        "UPDATE jobs SET status = $2, runs = $3, agent_id = $4, error = $5, started_at = $6,
                         finished_at = $7, approval = $8, approved_by = $9, approved_at = $10,
                         rejected_by = $11, rejected_at = $12, version = version + 1
         WHERE id = $1 RETURNING version",
        job.id.to_db(),
        job.status.as_str(),
        json::runs(&job.runs),
        job.agent_id.map(AppId::to_db),
        job.error,
        job.started_at,
        job.finished_at,
        approval,
        approved_by,
        approved_at,
        rejected_by,
        rejected_at,
    )
    .fetch_one(&mut **tx)
    .await?;
    job.version = version;
    Ok(())
}

/// Deletes a job with its logs, and returns it, or `None` if no job has this id.
pub(crate) async fn delete(
    tx: &mut Tx<'_>,
    id: JobId,
) -> Result<Option<(WorkspaceId, Option<AppId>, JobStatus)>, Error> {
    let row = sqlx::query!(
        "DELETE FROM jobs WHERE id = $1 RETURNING workspace_id, agent_id, status",
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|row| {
        let status = JobStatus::parse(&row.status).ok_or_else(|| bad_status(&row.status))?;
        Ok((
            WorkspaceId::from_db(row.workspace_id),
            row.agent_id.map(AppId::from_db),
            status,
        ))
    })
    .transpose()
}

/// Gives the oldest pending job of a workspace with no agent to an agent, and returns it: of
/// each project, or of `projects` only. A job with a container goes only to an agent with a
/// container runtime.
pub(crate) async fn claim(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    projects: Option<&[ProjectId]>,
    agent: AppId,
    containers: bool,
) -> Result<Option<Job>, Error> {
    let projects: Option<Vec<_>> = projects.map(|ids| ids.iter().map(|id| id.to_db()).collect());
    let claimed = sqlx::query_scalar!(
        "UPDATE jobs SET agent_id = $2, version = version + 1
         WHERE id = (SELECT id FROM jobs
                     WHERE workspace_id = $1 AND status = 'pending' AND agent_id IS NULL
                       AND ($3 OR NOT need_container)
                       AND ($4::uuid[] IS NULL OR project_id = ANY($4))
                     ORDER BY id LIMIT 1 FOR UPDATE SKIP LOCKED)
         RETURNING id",
        workspace.to_db(),
        agent.to_db(),
        containers,
        projects.as_deref(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    match claimed {
        Some(id) => find(tx, JobId::from_db(id), false).await,
        None => Ok(None),
    }
}
