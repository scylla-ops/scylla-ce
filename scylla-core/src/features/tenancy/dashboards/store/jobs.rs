//! The open items of the `jobs` table: the jobs that failed or that lost their agent, the jobs
//! that wait for an approval, and the times when such items went away.

use scylla_domain::identity::apps::AppId;
use scylla_domain::projects::jobs::{JobId, JobStatus, NodeRun};
use scylla_domain::projects::pipelines::{Node, PipelineId};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::dashboards::attention::{
    AttentionItem, AttentionKind, AttentionQuery, ItemJob, Place, Reach, Subject,
};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::features::projects::pipelines::json;
use crate::Error;

/// A row of a job item.
struct Row {
    /// The id of the job.
    id: Uuid,
    /// The pipeline.
    pipeline_id: Uuid,
    /// The display name of the pipeline.
    pipeline_name: String,
    /// The project.
    project_id: Uuid,
    /// The display name of the project.
    project_name: String,
    /// The state of the job.
    status: String,
    /// The error of the job.
    error: Option<String>,
    /// The branch of the commit of the job.
    branch: Option<String>,
    /// The agent of the job.
    agent_id: Option<Uuid>,
    /// The display name of the agent.
    agent_name: Option<String>,
    /// The time of the item.
    at: OffsetDateTime,
    /// The open items of the same pipeline, branch and kind.
    count: i64,
}

impl Row {
    /// Makes the item of the row.
    fn into_item(self) -> Result<AttentionItem, Error> {
        let bad = || Error::Internal(format!("bad job status: {}", self.status));
        let status = JobStatus::parse(&self.status).ok_or_else(bad)?;
        let kind = match status {
            JobStatus::Failed => AttentionKind::JobFailed,
            JobStatus::Orphaned => AttentionKind::JobOrphaned,
            JobStatus::AwaitingApproval => AttentionKind::ApprovalWaiting,
            _ => return Err(bad()),
        };
        let agent = self.agent_id.zip(self.agent_name);
        Ok(AttentionItem {
            kind,
            at: self.at,
            count: u32::try_from(self.count).unwrap_or(u32::MAX),
            is_new: false,
            place: Some(Place {
                project_id: ProjectId::from_db(self.project_id),
                project_name: self.project_name,
                pipeline_id: PipelineId::from_db(self.pipeline_id),
                pipeline_name: self.pipeline_name,
            }),
            subject: Subject::Job(ItemJob {
                id: JobId::from_db(self.id),
                status,
                error: self.error,
                branch: self.branch,
                agent: agent.map(|(id, name)| (AppId::from_db(id), name)),
                node: None,
            }),
        })
    }
}

/// The columns of a reach and of a filter of projects: each project, the projects of the reach,
/// and the projects of the filter.
pub(super) fn places(reach: &Reach, query: &AttentionQuery) -> (bool, Vec<Uuid>, Vec<Uuid>) {
    let ids = |ids: &[ProjectId]| ids.iter().map(|id| id.to_db()).collect();
    (
        reach.is_workspace(),
        ids(reach.projects()),
        ids(&query.projects),
    )
}

/// Returns the projects of `ids` that are in a workspace.
pub(crate) async fn projects_in(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    ids: &[ProjectId],
) -> Result<Vec<ProjectId>, sqlx::Error> {
    let ids: Vec<Uuid> = ids.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query_scalar!(
        "SELECT id FROM projects WHERE workspace_id = $1 AND id = ANY($2)",
        workspace.to_db(),
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(ProjectId::from_db).collect())
}

/// Returns the newest job of each pipeline, branch and state of `statuses` that ended at `from`
/// or later, with no later completed job of the same pipeline and branch. An archived pipeline
/// has none.
pub(crate) async fn failures(
    tx: &mut Tx<'_>,
    query: &AttentionQuery,
    statuses: &[&str],
    from: OffsetDateTime,
) -> Result<Vec<AttentionItem>, Error> {
    let (all, visible, projects) = places(&query.reach.jobs, query);
    let rows = sqlx::query_as!(
        Row,
        r#"WITH bad AS (
               SELECT j.id, j.pipeline_id, j.project_id, j.status, j.error, j.agent_id,
                      j.finished_at, s.branch, row_number() OVER w AS rank,
                      count(*) OVER w AS count
               FROM jobs j
               JOIN pipelines p ON p.id = j.pipeline_id
               LEFT JOIN job_sources s ON s.job_id = j.id
               WHERE j.workspace_id = $1 AND j.status = ANY($2::text[]) AND j.finished_at >= $3
                 AND p.archived_at IS NULL AND p.removed_at IS NULL
                 AND ($4::bool OR j.project_id = ANY($5::uuid[]))
                 AND (cardinality($6::uuid[]) = 0 OR j.project_id = ANY($6))
                 AND NOT EXISTS (
                     SELECT 1 FROM jobs c LEFT JOIN job_sources cs ON cs.job_id = c.id
                     WHERE c.pipeline_id = j.pipeline_id AND c.id > j.id
                       AND c.status = 'completed' AND cs.branch IS NOT DISTINCT FROM s.branch)
               WINDOW w AS (PARTITION BY j.pipeline_id, s.branch, j.status ORDER BY j.id DESC
                             ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING)
           )
           SELECT b.id AS "id!", b.pipeline_id AS "pipeline_id!", p.name AS pipeline_name,
                  b.project_id AS "project_id!", r.name AS project_name, b.status AS "status!",
                  b.error AS "error?", b.branch AS "branch?", b.agent_id AS "agent_id?",
                  a.name AS "agent_name?", b.finished_at AS "at!", b.count AS "count!"
           FROM bad b
           JOIN pipelines p ON p.id = b.pipeline_id
           JOIN projects r ON r.id = b.project_id
           LEFT JOIN apps a ON a.id = b.agent_id
           WHERE b.rank = 1"#,
        query.workspace.to_db(),
        statuses as &[&str],
        from,
        all,
        &visible,
        &projects,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter().map(Row::into_item).collect()
}

/// Returns the jobs that wait for an approval.
pub(crate) async fn approvals(
    tx: &mut Tx<'_>,
    query: &AttentionQuery,
) -> Result<Vec<AttentionItem>, Error> {
    let (all, visible, projects) = places(&query.reach.approvals, query);
    let rows = sqlx::query_as!(
        Row,
        r#"SELECT j.id, j.pipeline_id, p.name AS pipeline_name, j.project_id,
                  r.name AS project_name, j.status, j.error, s.branch AS "branch?",
                  j.agent_id, a.name AS "agent_name?", j.created_at AS at, 1::int8 AS "count!"
           FROM jobs j
           JOIN pipelines p ON p.id = j.pipeline_id
           JOIN projects r ON r.id = j.project_id
           LEFT JOIN job_sources s ON s.job_id = j.id
           LEFT JOIN apps a ON a.id = j.agent_id
           WHERE j.workspace_id = $1 AND j.status = 'awaiting_approval'
             AND ($2::bool OR j.project_id = ANY($3::uuid[]))
             AND (cardinality($4::uuid[]) = 0 OR j.project_id = ANY($4))"#,
        query.workspace.to_db(),
        all,
        &visible,
        &projects,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter().map(Row::into_item).collect()
}

/// Returns the last end of a completed job that came after a job of `statuses` of the same
/// pipeline and branch, from `from`.
pub(crate) async fn failures_cleared(
    tx: &mut Tx<'_>,
    query: &AttentionQuery,
    statuses: &[&str],
    from: OffsetDateTime,
) -> Result<Option<OffsetDateTime>, sqlx::Error> {
    let (all, visible, projects) = places(&query.reach.jobs, query);
    sqlx::query_scalar!(
        r#"SELECT max(t.finished_at) AS at FROM (
               SELECT j.status, j.finished_at,
                      lag(j.status) OVER (PARTITION BY j.pipeline_id, s.branch ORDER BY j.id)
                          AS before
               FROM jobs j
               JOIN pipelines p ON p.id = j.pipeline_id
               LEFT JOIN job_sources s ON s.job_id = j.id
               WHERE j.workspace_id = $1 AND j.finished_at >= $3
                 AND j.status IN ('completed', 'failed', 'orphaned')
                 AND p.archived_at IS NULL AND p.removed_at IS NULL
                 AND ($4::bool OR j.project_id = ANY($5::uuid[]))
                 AND (cardinality($6::uuid[]) = 0 OR j.project_id = ANY($6))
           ) t
           WHERE t.status = 'completed' AND t.before = ANY($2::text[])"#,
        query.workspace.to_db(),
        statuses as &[&str],
        from,
        all,
        &visible,
        &projects,
    )
    .fetch_one(&mut **tx)
    .await
}

/// Returns the last approval, rejection or replacement of a job from a fork that was created at
/// `from` or later.
pub(crate) async fn approvals_cleared(
    tx: &mut Tx<'_>,
    query: &AttentionQuery,
    from: OffsetDateTime,
) -> Result<Option<OffsetDateTime>, sqlx::Error> {
    let (all, visible, projects) = places(&query.reach.approvals, query);
    sqlx::query_scalar!(
        r#"SELECT max(CASE j.approval WHEN 'approved' THEN j.approved_at
                                      WHEN 'rejected' THEN j.rejected_at
                                      ELSE j.finished_at END) AS at
           FROM jobs j
           WHERE j.workspace_id = $1 AND j.created_at >= $2 AND j.approval IS NOT NULL
             AND j.status <> 'awaiting_approval'
             AND ($3::bool OR j.project_id = ANY($4::uuid[]))
             AND (cardinality($5::uuid[]) = 0 OR j.project_id = ANY($5))"#,
        query.workspace.to_db(),
        from,
        all,
        &visible,
        &projects,
    )
    .fetch_one(&mut **tx)
    .await
}

/// Returns the nodes and the states of the nodes of some jobs.
pub(crate) async fn nodes(
    tx: &mut Tx<'_>,
    ids: &[JobId],
) -> Result<Vec<(JobId, Vec<Node>, Vec<NodeRun>)>, Error> {
    let ids: Vec<Uuid> = ids.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query!("SELECT id, nodes, runs FROM jobs WHERE id = ANY($1)", &ids)
        .fetch_all(&mut **tx)
        .await?;
    rows.into_iter()
        .map(|row| {
            let (nodes, runs): (Value, Value) = (row.nodes, row.runs);
            Ok((
                JobId::from_db(row.id),
                json::read_nodes(nodes)?,
                json::read_runs(runs)?,
            ))
        })
        .collect()
}
