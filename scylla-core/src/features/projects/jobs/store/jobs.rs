//! The SQL of the `jobs` table.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::apps::AppId;
use scylla_domain::projects::jobs::{Job, JobId, JobScope, JobStatus, NewJob, NodeRun};
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::PageRequest;
use scylla_extension::Tx;
use serde_json::Value;
use time::OffsetDateTime;

use crate::features::projects::pipelines::json;
use crate::Error;

/// Inserts a pending job and returns it.
pub(crate) async fn insert(tx: &mut Tx<'_>, new: &NewJob) -> Result<Job, Error> {
    let runs: Vec<NodeRun> = new.nodes.iter().map(NodeRun::pending).collect();
    let need_container = new.nodes.iter().any(|node| {
        !matches!(
            node.runtime,
            scylla_domain::projects::pipelines::Runtime::Process
        )
    });
    let row = sqlx::query_as!(
        Row,
        "INSERT INTO jobs (id, pipeline_id, project_id, workspace_id, created_by, rerun_of, status,
                           nodes, runs, need_container)
         VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7, $8, $9)
         RETURNING id, pipeline_id, project_id, workspace_id, created_by, rerun_of, status, nodes,
                   runs, agent_id, error, version, created_at, started_at, finished_at",
        JobId::generate().to_db(),
        new.pipeline_id.to_db(),
        new.project_id.to_db(),
        new.workspace_id.to_db(),
        new.created_by.to_db(),
        new.rerun_of.map(JobId::to_db),
        json::nodes(&new.nodes),
        json::runs(&runs),
        need_container,
    )
    .fetch_one(&mut **tx)
    .await?;
    row.into_job()
}

/// Returns a job, or `None` if no job has this id. With `lock`, the row stays locked until the
/// end of the transaction.
pub(crate) async fn find(tx: &mut Tx<'_>, id: JobId, lock: bool) -> Result<Option<Job>, Error> {
    let row = if lock {
        sqlx::query_as!(
            Row,
            "SELECT id, pipeline_id, project_id, workspace_id, created_by, rerun_of, status, nodes,
                    runs, agent_id, error, version, created_at, started_at, finished_at
             FROM jobs WHERE id = $1 FOR UPDATE",
            id.to_db(),
        )
        .fetch_optional(&mut **tx)
        .await?
    } else {
        sqlx::query_as!(
            Row,
            "SELECT id, pipeline_id, project_id, workspace_id, created_by, rerun_of, status, nodes,
                    runs, agent_id, error, version, created_at, started_at, finished_at
             FROM jobs WHERE id = $1",
            id.to_db(),
        )
        .fetch_optional(&mut **tx)
        .await?
    };
    row.map(Row::into_job).transpose()
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

/// Returns the jobs of a pipeline or of a workspace before the page id, newest first, with one
/// extra row.
pub(crate) async fn list(
    tx: &mut Tx<'_>,
    scope: JobScope,
    page: PageRequest<JobId>,
) -> Result<Vec<Job>, Error> {
    let (pipeline, workspace, agent) = match scope {
        JobScope::Pipeline(id) => (Some(id.to_db()), None, None),
        JobScope::Workspace(id, agent) => (None, Some(id.to_db()), agent.map(AppId::to_db)),
    };
    let rows = sqlx::query_as!(
        Row,
        "SELECT id, pipeline_id, project_id, workspace_id, created_by, rerun_of, status, nodes,
                runs, agent_id, error, version, created_at, started_at, finished_at
         FROM jobs
         WHERE ($1::uuid IS NULL OR pipeline_id = $1)
           AND ($2::uuid IS NULL OR workspace_id = $2)
           AND ($3::uuid IS NULL OR agent_id = $3)
           AND ($4::uuid IS NULL OR id < $4)
         ORDER BY id DESC
         LIMIT $5",
        pipeline,
        workspace,
        agent,
        page.after.map(JobId::to_db),
        page.size.get() as i64 + 1,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter().map(Row::into_job).collect()
}

/// Writes the state of a job that the caller read in this transaction, with one more version.
pub(crate) async fn save(tx: &mut Tx<'_>, job: &mut Job) -> Result<(), Error> {
    let version = sqlx::query_scalar!(
        "UPDATE jobs SET status = $2, runs = $3, agent_id = $4, error = $5, started_at = $6,
                         finished_at = $7, version = version + 1
         WHERE id = $1 RETURNING version",
        job.id.to_db(),
        job.status.as_str(),
        json::runs(&job.runs),
        job.agent_id.map(AppId::to_db),
        job.error,
        job.started_at,
        job.finished_at,
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

/// Gives the oldest pending job of a workspace with no agent to an agent, and returns it. A job
/// with a container goes only to an agent with a container runtime.
pub(crate) async fn claim(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    agent: AppId,
    containers: bool,
) -> Result<Option<Job>, Error> {
    let row = sqlx::query_as!(
        Row,
        "UPDATE jobs SET agent_id = $2, version = version + 1
         WHERE id = (SELECT id FROM jobs
                     WHERE workspace_id = $1 AND status = 'pending' AND agent_id IS NULL
                       AND ($3 OR NOT need_container)
                     ORDER BY id LIMIT 1 FOR UPDATE SKIP LOCKED)
         RETURNING id, pipeline_id, project_id, workspace_id, created_by, rerun_of, status, nodes,
                   runs, agent_id, error, version, created_at, started_at, finished_at",
        workspace.to_db(),
        agent.to_db(),
        containers,
    )
    .fetch_optional(&mut **tx)
    .await?;
    row.map(Row::into_job).transpose()
}

/// The error of a state that the code does not know.
fn bad_status(status: &str) -> Error {
    Error::Internal(format!("unknown job status {status}"))
}

/// A row of the `jobs` table.
struct Row {
    /// The id.
    id: uuid::Uuid,
    /// The pipeline.
    pipeline_id: uuid::Uuid,
    /// The project.
    project_id: uuid::Uuid,
    /// The workspace.
    workspace_id: uuid::Uuid,
    /// The author.
    created_by: uuid::Uuid,
    /// The job that this job runs again.
    rerun_of: Option<uuid::Uuid>,
    /// The state.
    status: String,
    /// The nodes in JSON.
    nodes: Value,
    /// The states of the nodes in JSON.
    runs: Value,
    /// The agent.
    agent_id: Option<uuid::Uuid>,
    /// The error.
    error: Option<String>,
    /// The version.
    version: i64,
    /// The creation time.
    created_at: OffsetDateTime,
    /// The start.
    started_at: Option<OffsetDateTime>,
    /// The end.
    finished_at: Option<OffsetDateTime>,
}

impl Row {
    /// Makes the job of the row.
    fn into_job(self) -> Result<Job, Error> {
        Ok(Job {
            id: JobId::from_db(self.id),
            pipeline_id: PipelineId::from_db(self.pipeline_id),
            project_id: ProjectId::from_db(self.project_id),
            workspace_id: WorkspaceId::from_db(self.workspace_id),
            created_by: ActorId::from_db(self.created_by),
            rerun_of: self.rerun_of.map(JobId::from_db),
            status: JobStatus::parse(&self.status).ok_or_else(|| bad_status(&self.status))?,
            nodes: json::read_nodes(self.nodes)?,
            runs: json::read_runs(self.runs)?,
            agent_id: self.agent_id.map(AppId::from_db),
            error: self.error,
            version: self.version,
            created_at: self.created_at,
            started_at: self.started_at,
            finished_at: self.finished_at,
        })
    }
}
