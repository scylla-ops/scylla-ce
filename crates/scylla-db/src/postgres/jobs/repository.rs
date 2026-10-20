use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{AppId, JobId, OrganizationId, PipelineId, ProjectId, StreamId};
use crate::domain::job::JobState;
use crate::domain::job::{Job, JobNode};
use crate::domain::job::{JobOrigin, JobStatus};
use crate::domain::pipeline::{EnvKey, EnvValue, PipelineNode};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scylla_auth::authz::Visibility;
use scylla_core::application::agent::AgentStream;
use scylla_core::application::pagination::{PaginatedResult, PaginationParams};
use scylla_core::application::{JobRepository, job::JobScope};
use sqlx::{PgExecutor, PgPool, types::Json};
use tracing::instrument;

use super::super::error::{DbFieldExt, SqlxResultExt};
use super::super::version::{from_db, to_db, written};
use super::super::visibility::VisibilityFilter;

#[derive(Clone)]
pub struct PgJobRepository {
    pool: PgPool,
}

impl PgJobRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl JobRepository for PgJobRepository {
    #[instrument(skip_all, fields(job_id = %job.id()))]
    async fn create(&self, job: &Job) -> DomainResult<Job> {
        queries::create(&self.pool, job).await
    }

    #[instrument(skip_all, fields(job_id = %id))]
    async fn find_by_id(&self, id: &JobId) -> DomainResult<Job> {
        queries::find_by_id(&self.pool, id).await
    }

    #[instrument(skip_all, fields(job_id = %job.id(), version = job.version()))]
    async fn update(&self, job: &Job) -> DomainResult<Job> {
        let updated = queries::update(&self.pool, job).await?;
        written(
            updated,
            "Job",
            job.id(),
            queries::find_by_id(&self.pool, job.id()),
        )
        .await
    }

    #[instrument(skip_all, fields(job_id = %job.id(), version = job.version()))]
    async fn delete(&self, job: &Job) -> DomainResult<()> {
        let deleted = queries::delete(&self.pool, job).await?.then_some(());
        written(
            deleted,
            "Job",
            job.id(),
            queries::find_by_id(&self.pool, job.id()),
        )
        .await
    }

    #[instrument(skip_all, fields(app_id = %stream.agent, stream_id = %stream.id))]
    async fn claim_next(
        &self,
        stream: &AgentStream,
        visible: &Visibility,
    ) -> DomainResult<Option<(Job, ProjectId)>> {
        queries::claim_next(&self.pool, stream, &VisibilityFilter::new(visible)).await
    }

    #[instrument(skip_all, fields(app_id = %stream.agent, stream_id = %stream.id))]
    async fn release(&self, stream: &AgentStream) -> DomainResult<u64> {
        queries::release(&self.pool, stream).await
    }

    #[instrument(skip_all)]
    async fn pending_streams(&self) -> DomainResult<Vec<AgentStream>> {
        queries::pending_streams(&self.pool).await
    }

    #[instrument(skip_all, fields(agents = agents.len()))]
    async fn active_jobs(&self, agents: &[AppId]) -> DomainResult<Vec<(AppId, u32)>> {
        queries::active_jobs(&self.pool, agents).await
    }

    #[instrument(skip_all)]
    async fn list_live(&self, scope: JobScope<'_>) -> DomainResult<Vec<Job>> {
        queries::list_live(&self.pool, scope).await
    }

    #[instrument(skip_all, fields(app_id = %agent))]
    async fn list_running_on(&self, agent: &AppId) -> DomainResult<Vec<Job>> {
        queries::list_running_on(&self.pool, agent).await
    }

    #[instrument(skip_all)]
    async fn list_stranded(&self, seen_before: DateTime<Utc>) -> DomainResult<Vec<Job>> {
        queries::list_stranded(&self.pool, seen_before).await
    }

    #[instrument(skip(self, pagination))]
    async fn list_all(
        &self,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Job>> {
        let params = pagination.copied().unwrap_or_default();
        let total = queries::count(&self.pool, JobScope::All, None).await?;
        let items = queries::list_page(&self.pool, &params, JobScope::All, None).await?;
        Ok(PaginatedResult::new(items, &params, total))
    }

    #[instrument(skip_all, fields(pipeline_id = %pipeline_id))]
    async fn list_by_pipeline(
        &self,
        pipeline_id: &PipelineId,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Job>> {
        let params = pagination.copied().unwrap_or_default();
        let scope = JobScope::Pipeline(pipeline_id);
        let total = queries::count(&self.pool, scope, None).await?;
        let items = queries::list_page(&self.pool, &params, scope, None).await?;
        Ok(PaginatedResult::new(items, &params, total))
    }

    #[instrument(skip_all, fields(project_id = %project_id))]
    async fn list_by_project(
        &self,
        project_id: &ProjectId,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Job>> {
        let params = pagination.copied().unwrap_or_default();
        let scope = JobScope::Project(project_id);
        let total = queries::count(&self.pool, scope, None).await?;
        let items = queries::list_page(&self.pool, &params, scope, None).await?;
        Ok(PaginatedResult::new(items, &params, total))
    }

    #[instrument(skip_all, fields(org_id = %organization_id))]
    async fn list_by_organization(
        &self,
        organization_id: &OrganizationId,
        agent: Option<&AppId>,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Job>> {
        let params = pagination.copied().unwrap_or_default();
        let scope = JobScope::Organization(organization_id);
        let total = queries::count(&self.pool, scope, agent).await?;
        let items = queries::list_page(&self.pool, &params, scope, agent).await?;
        Ok(PaginatedResult::new(items, &params, total))
    }
}

#[derive(sqlx::FromRow)]
struct JobRow {
    id: String,
    pipeline_id: String,
    status: String,
    nodes: Json<Vec<PipelineNode>>,
    node_executions: Json<Vec<JobNode>>,
    inputs: Json<Vec<(EnvKey, EnvValue)>>,
    origin: Json<JobOrigin>,
    agent_app_id: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    version: i64,
}

impl TryFrom<JobRow> for Job {
    type Error = DomainError;
    fn try_from(r: JobRow) -> DomainResult<Self> {
        let status = JobStatus::new(r.status).db_field("job status")?;
        let state =
            JobState::from_columns(status, r.started_at, r.finished_at).db_field("job state")?;
        Ok(Job::from_persistence(
            JobId::new(r.id),
            PipelineId::new(r.pipeline_id),
            state,
            r.agent_app_id.map(AppId::new),
            r.nodes.0,
            r.node_executions.0,
            r.inputs.0,
            r.origin.0,
            r.created_at,
            r.updated_at,
            from_db(r.version),
        ))
    }
}

#[allow(clippy::wildcard_imports)]
pub mod queries {
    use super::*;

    pub async fn create<'e, E>(executor: E, job: &Job) -> DomainResult<Job>
    where
        E: PgExecutor<'e>,
    {
        let nodes = Json(job.nodes().to_vec());
        let executions = Json(job.node_executions().to_vec());
        let inputs = Json(job.inputs().to_vec());
        let origin = Json(job.origin().clone());
        sqlx::query!(
            r#"
            INSERT INTO jobs (id, pipeline_id, status, nodes, node_executions, inputs, origin, agent_app_id, created_at, updated_at, started_at, finished_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            "#,
            job.id().as_str(),
            job.pipeline_id().as_str(),
            job.status().as_str(),
            nodes as _,
            executions as _,
            inputs as _,
            origin as _,
            job.agent_app_id().map(AppId::as_str),
            job.created_at(),
            job.updated_at(),
            job.started_at(),
            job.finished_at(),
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(job.clone())
    }

    pub async fn find_by_id<'e, E>(executor: E, id: &JobId) -> DomainResult<Job>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query_as!(
            JobRow,
            r#"
            SELECT id, pipeline_id, status,
                   nodes AS "nodes: Json<Vec<PipelineNode>>",
                   node_executions AS "node_executions: Json<Vec<JobNode>>",
                   inputs AS "inputs: Json<Vec<(EnvKey, EnvValue)>>",
                   origin AS "origin: Json<JobOrigin>",
                   agent_app_id,
                   created_at, updated_at, started_at, finished_at, version
            FROM jobs
            WHERE id = $1
            "#,
            id.as_str(),
        )
        .fetch_one(executor)
        .await
        .not_found_as("Job", id)?
        .try_into()
    }

    /// `None` when no row carries the staged version. The agent is the store's: the update
    /// leaves it as it is.
    pub async fn update<'e, E>(executor: E, job: &Job) -> DomainResult<Option<Job>>
    where
        E: PgExecutor<'e>,
    {
        let executions = Json(job.node_executions().to_vec());
        sqlx::query_as!(
            JobRow,
            r#"
            UPDATE jobs
            SET status = $2,
                node_executions = $3,
                updated_at = $4,
                started_at = $5,
                finished_at = $6,
                version = version + 1
            WHERE id = $1 AND version = $7
            RETURNING id, pipeline_id, status,
                      nodes AS "nodes: Json<Vec<PipelineNode>>",
                      node_executions AS "node_executions: Json<Vec<JobNode>>",
                      inputs AS "inputs: Json<Vec<(EnvKey, EnvValue)>>",
                      origin AS "origin: Json<JobOrigin>",
                      agent_app_id,
                      created_at, updated_at, started_at, finished_at, version
            "#,
            job.id().as_str(),
            job.status().as_str(),
            executions as _,
            job.updated_at(),
            job.started_at(),
            job.finished_at(),
            to_db(job.version()),
        )
        .fetch_optional(executor)
        .await
        .to_domain()?
        .map(Job::try_from)
        .transpose()
    }

    /// `false` when no row carries the staged version.
    pub async fn delete<'e, E>(executor: E, job: &Job) -> DomainResult<bool>
    where
        E: PgExecutor<'e>,
    {
        let res = sqlx::query!(
            "DELETE FROM jobs WHERE id = $1 AND version = $2",
            job.id().as_str(),
            to_db(job.version()),
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(res.rows_affected() > 0)
    }

    /// One statement: the row lock and `SKIP LOCKED` make two passes place a job once, and the
    /// agent must have no job that has not ended.
    pub async fn claim_next<'e, E>(
        executor: E,
        stream: &AgentStream,
        visible: &VisibilityFilter,
    ) -> DomainResult<Option<(Job, ProjectId)>>
    where
        E: PgExecutor<'e>,
    {
        let claimed = sqlx::query!(
            r#"
            WITH claimed AS (
                UPDATE jobs
                SET agent_app_id = $1, stream_id = $2, version = version + 1, updated_at = now()
                WHERE id = (
                    SELECT j.id
                    FROM jobs j
                    JOIN pipelines p ON p.id = j.pipeline_id
                    JOIN projects pr ON pr.id = p.project_id
                    WHERE j.status = 'pending'
                      AND j.agent_app_id IS NULL
                      AND ($3 OR p.project_id = ANY($4) OR pr.organization_id = ANY($5))
                      AND NOT EXISTS (
                          SELECT 1 FROM jobs b
                          WHERE b.agent_app_id = $1 AND b.status IN ('pending', 'running')
                      )
                    ORDER BY j.created_at
                    LIMIT 1
                    FOR UPDATE OF j SKIP LOCKED
                )
                AND status = 'pending'
                AND agent_app_id IS NULL
                RETURNING *
            )
            SELECT c.id AS "id!", c.pipeline_id AS "pipeline_id!", c.status AS "status!",
                   c.nodes AS "nodes!: Json<Vec<PipelineNode>>",
                   c.node_executions AS "node_executions!: Json<Vec<JobNode>>",
                   c.inputs AS "inputs!: Json<Vec<(EnvKey, EnvValue)>>",
                   c.origin AS "origin!: Json<JobOrigin>",
                   c.agent_app_id,
                   c.created_at AS "created_at!", c.updated_at AS "updated_at!",
                   c.started_at, c.finished_at, c.version AS "version!",
                   p.project_id AS "project_id!"
            FROM claimed c
            JOIN pipelines p ON p.id = c.pipeline_id
            "#,
            stream.agent.as_str(),
            stream.id.as_str(),
            visible.all,
            &visible.projects,
            &visible.orgs,
        )
        .fetch_optional(executor)
        .await
        .to_domain()?;
        let Some(r) = claimed else {
            return Ok(None);
        };
        let job = Job::try_from(JobRow {
            id: r.id,
            pipeline_id: r.pipeline_id,
            status: r.status,
            nodes: r.nodes,
            node_executions: r.node_executions,
            inputs: r.inputs,
            origin: r.origin,
            agent_app_id: r.agent_app_id,
            created_at: r.created_at,
            updated_at: r.updated_at,
            started_at: r.started_at,
            finished_at: r.finished_at,
            version: r.version,
        })?;
        Ok(Some((job, ProjectId::new(r.project_id))))
    }

    pub async fn active_jobs<'e, E>(
        executor: E,
        agents: &[AppId],
    ) -> DomainResult<Vec<(AppId, u32)>>
    where
        E: PgExecutor<'e>,
    {
        let rows = sqlx::query!(
            r#"
            SELECT agent_app_id AS "agent_app_id!", COUNT(*) AS "count!"
            FROM jobs
            WHERE agent_app_id = ANY($1) AND status IN ('pending', 'running')
            GROUP BY agent_app_id
            "#,
            &agents
                .iter()
                .map(|a| a.as_str().to_owned())
                .collect::<Vec<_>>(),
        )
        .fetch_all(executor)
        .await
        .to_domain()?;
        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    AppId::new(r.agent_app_id),
                    u32::try_from(r.count).unwrap_or(u32::MAX),
                )
            })
            .collect())
    }

    pub async fn release<'e, E>(executor: E, stream: &AgentStream) -> DomainResult<u64>
    where
        E: PgExecutor<'e>,
    {
        let res = sqlx::query!(
            r#"
            UPDATE jobs
            SET agent_app_id = NULL, stream_id = NULL, version = version + 1, updated_at = now()
            WHERE agent_app_id = $1 AND stream_id = $2 AND status = 'pending'
            "#,
            stream.agent.as_str(),
            stream.id.as_str(),
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(res.rows_affected())
    }

    pub async fn pending_streams<'e, E>(executor: E) -> DomainResult<Vec<AgentStream>>
    where
        E: PgExecutor<'e>,
    {
        let rows = sqlx::query!(
            r#"
            SELECT DISTINCT agent_app_id AS "agent_app_id!", stream_id AS "stream_id!"
            FROM jobs
            WHERE status = 'pending' AND agent_app_id IS NOT NULL AND stream_id IS NOT NULL
            "#,
        )
        .fetch_all(executor)
        .await
        .to_domain()?;
        Ok(rows
            .into_iter()
            .map(|r| AgentStream {
                agent: AppId::new(r.agent_app_id),
                id: StreamId::new(r.stream_id),
            })
            .collect())
    }

    pub async fn list_live<'e, E>(executor: E, scope: JobScope<'_>) -> DomainResult<Vec<Job>>
    where
        E: PgExecutor<'e>,
    {
        let (pipeline, project, organization) = match scope {
            JobScope::All => (None, None, None),
            JobScope::Pipeline(id) => (Some(id.as_str()), None, None),
            JobScope::Project(id) => (None, Some(id.as_str()), None),
            JobScope::Organization(id) => (None, None, Some(id.as_str())),
        };
        let rows: Vec<JobRow> = sqlx::query_as!(
            JobRow,
            r#"
            SELECT j.id, j.pipeline_id, j.status,
                   j.nodes AS "nodes: Json<Vec<PipelineNode>>",
                   j.node_executions AS "node_executions: Json<Vec<JobNode>>",
                   j.inputs AS "inputs: Json<Vec<(EnvKey, EnvValue)>>",
                   j.origin AS "origin: Json<JobOrigin>",
                   j.agent_app_id,
                   j.created_at, j.updated_at, j.started_at, j.finished_at, j.version
            FROM jobs j
            JOIN pipelines p ON p.id = j.pipeline_id
            JOIN projects pr ON pr.id = p.project_id
            WHERE j.status IN ('pending', 'running')
              AND ($1::text IS NULL OR j.pipeline_id = $1)
              AND ($2::text IS NULL OR p.project_id = $2)
              AND ($3::text IS NULL OR pr.organization_id = $3)
            "#,
            pipeline,
            project,
            organization,
        )
        .fetch_all(executor)
        .await
        .to_domain()?;
        rows.into_iter().map(Job::try_from).collect()
    }

    pub async fn list_running_on<'e, E>(executor: E, agent: &AppId) -> DomainResult<Vec<Job>>
    where
        E: PgExecutor<'e>,
    {
        let rows: Vec<JobRow> = sqlx::query_as!(
            JobRow,
            r#"
            SELECT id, pipeline_id, status,
                   nodes AS "nodes: Json<Vec<PipelineNode>>",
                   node_executions AS "node_executions: Json<Vec<JobNode>>",
                   inputs AS "inputs: Json<Vec<(EnvKey, EnvValue)>>",
                   origin AS "origin: Json<JobOrigin>",
                   agent_app_id,
                   created_at, updated_at, started_at, finished_at, version
            FROM jobs
            WHERE agent_app_id = $1 AND status = 'running'
            "#,
            agent.as_str(),
        )
        .fetch_all(executor)
        .await
        .to_domain()?;
        rows.into_iter().map(Job::try_from).collect()
    }

    pub async fn list_stranded<'e, E>(
        executor: E,
        seen_before: DateTime<Utc>,
    ) -> DomainResult<Vec<Job>>
    where
        E: PgExecutor<'e>,
    {
        let rows: Vec<JobRow> = sqlx::query_as!(
            JobRow,
            r#"
            SELECT j.id, j.pipeline_id, j.status,
                   j.nodes AS "nodes: Json<Vec<PipelineNode>>",
                   j.node_executions AS "node_executions: Json<Vec<JobNode>>",
                   j.inputs AS "inputs: Json<Vec<(EnvKey, EnvValue)>>",
                   j.origin AS "origin: Json<JobOrigin>",
                   j.agent_app_id,
                   j.created_at, j.updated_at, j.started_at, j.finished_at, j.version
            FROM jobs j
            LEFT JOIN agents a ON a.app_id = j.agent_app_id
            WHERE j.status = 'running' AND (a.last_seen IS NULL OR a.last_seen < $1)
            "#,
            seen_before,
        )
        .fetch_all(executor)
        .await
        .to_domain()?;
        rows.into_iter().map(Job::try_from).collect()
    }

    pub async fn count<'e, E>(
        executor: E,
        scope: JobScope<'_>,
        agent: Option<&AppId>,
    ) -> DomainResult<u64>
    where
        E: PgExecutor<'e>,
    {
        let agent = agent.map(AppId::as_str);
        let count: i64 = match scope {
            JobScope::All => sqlx::query_scalar!(
                r#"
                SELECT COUNT(*) AS "count!"
                FROM jobs
                WHERE ($1::text IS NULL OR agent_app_id = $1)
                "#,
                agent,
            )
            .fetch_one(executor)
            .await
            .to_domain()?,
            JobScope::Pipeline(pipeline_id) => sqlx::query_scalar!(
                r#"
                SELECT COUNT(*) AS "count!"
                FROM jobs
                WHERE pipeline_id = $1 AND ($2::text IS NULL OR agent_app_id = $2)
                "#,
                pipeline_id.as_str(),
                agent,
            )
            .fetch_one(executor)
            .await
            .to_domain()?,
            JobScope::Project(project_id) => sqlx::query_scalar!(
                r#"
                SELECT COUNT(*) AS "count!"
                FROM jobs j
                JOIN pipelines p ON p.id = j.pipeline_id
                WHERE p.project_id = $1 AND ($2::text IS NULL OR j.agent_app_id = $2)
                "#,
                project_id.as_str(),
                agent,
            )
            .fetch_one(executor)
            .await
            .to_domain()?,
            JobScope::Organization(org_id) => sqlx::query_scalar!(
                r#"
                SELECT COUNT(*) AS "count!"
                FROM jobs j
                JOIN pipelines p ON p.id = j.pipeline_id
                JOIN projects pr ON pr.id = p.project_id
                WHERE pr.organization_id = $1 AND ($2::text IS NULL OR j.agent_app_id = $2)
                "#,
                org_id.as_str(),
                agent,
            )
            .fetch_one(executor)
            .await
            .to_domain()?,
        };
        Ok(u64::try_from(count).unwrap_or(0))
    }

    #[allow(clippy::too_many_lines)] // four near-identical query_as! branches
    pub async fn list_page<'e, E>(
        executor: E,
        params: &PaginationParams,
        scope: JobScope<'_>,
        agent: Option<&AppId>,
    ) -> DomainResult<Vec<Job>>
    where
        E: PgExecutor<'e>,
    {
        let limit = i64::try_from(params.limit()).unwrap_or(i64::MAX);
        let offset = i64::try_from(params.offset()).unwrap_or(i64::MAX);
        let agent = agent.map(AppId::as_str);
        let rows: Vec<JobRow> = match scope {
            JobScope::All => sqlx::query_as!(
                JobRow,
                r#"
                SELECT id, pipeline_id, status,
                       nodes AS "nodes: Json<Vec<PipelineNode>>",
                       node_executions AS "node_executions: Json<Vec<JobNode>>",
                       inputs AS "inputs: Json<Vec<(EnvKey, EnvValue)>>",
                       origin AS "origin: Json<JobOrigin>",
                       agent_app_id,
                       created_at, updated_at, started_at, finished_at, version
                FROM jobs
                WHERE ($1::text IS NULL OR agent_app_id = $1)
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#,
                agent,
                limit,
                offset,
            )
            .fetch_all(executor)
            .await
            .to_domain()?,
            JobScope::Pipeline(pipeline_id) => sqlx::query_as!(
                JobRow,
                r#"
                SELECT id, pipeline_id, status,
                       nodes AS "nodes: Json<Vec<PipelineNode>>",
                       node_executions AS "node_executions: Json<Vec<JobNode>>",
                       inputs AS "inputs: Json<Vec<(EnvKey, EnvValue)>>",
                       origin AS "origin: Json<JobOrigin>",
                       agent_app_id,
                       created_at, updated_at, started_at, finished_at, version
                FROM jobs
                WHERE pipeline_id = $1 AND ($2::text IS NULL OR agent_app_id = $2)
                ORDER BY created_at DESC
                LIMIT $3 OFFSET $4
                "#,
                pipeline_id.as_str(),
                agent,
                limit,
                offset,
            )
            .fetch_all(executor)
            .await
            .to_domain()?,
            JobScope::Project(project_id) => sqlx::query_as!(
                JobRow,
                r#"
                SELECT j.id, j.pipeline_id, j.status,
                       j.nodes AS "nodes: Json<Vec<PipelineNode>>",
                       j.node_executions AS "node_executions: Json<Vec<JobNode>>",
                       j.inputs AS "inputs: Json<Vec<(EnvKey, EnvValue)>>",
                       j.origin AS "origin: Json<JobOrigin>",
                       j.agent_app_id,
                       j.created_at, j.updated_at, j.started_at, j.finished_at, j.version
                FROM jobs j
                JOIN pipelines p ON p.id = j.pipeline_id
                WHERE p.project_id = $1 AND ($2::text IS NULL OR j.agent_app_id = $2)
                ORDER BY j.created_at DESC
                LIMIT $3 OFFSET $4
                "#,
                project_id.as_str(),
                agent,
                limit,
                offset,
            )
            .fetch_all(executor)
            .await
            .to_domain()?,
            JobScope::Organization(org_id) => sqlx::query_as!(
                JobRow,
                r#"
                SELECT j.id, j.pipeline_id, j.status,
                       j.nodes AS "nodes: Json<Vec<PipelineNode>>",
                       j.node_executions AS "node_executions: Json<Vec<JobNode>>",
                       j.inputs AS "inputs: Json<Vec<(EnvKey, EnvValue)>>",
                       j.origin AS "origin: Json<JobOrigin>",
                       j.agent_app_id,
                       j.created_at, j.updated_at, j.started_at, j.finished_at, j.version
                FROM jobs j
                JOIN pipelines p ON p.id = j.pipeline_id
                JOIN projects pr ON pr.id = p.project_id
                WHERE pr.organization_id = $1 AND ($2::text IS NULL OR j.agent_app_id = $2)
                ORDER BY j.created_at DESC
                LIMIT $3 OFFSET $4
                "#,
                org_id.as_str(),
                agent,
                limit,
                offset,
            )
            .fetch_all(executor)
            .await
            .to_domain()?,
        };
        rows.into_iter().map(Job::try_from).collect()
    }
}
