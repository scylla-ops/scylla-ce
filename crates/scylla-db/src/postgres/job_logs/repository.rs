use crate::domain::errors::DomainResult;
use crate::domain::ids::{JobId, JobLogId};
use crate::domain::job::JobLog;
use crate::domain::job::LogStream;
use crate::domain::pipeline::NodeId;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scylla_core::application::JobLogRepository;
use scylla_core::application::pagination::{PaginatedResult, PaginationParams};
use sqlx::{PgExecutor, PgPool};
use tracing::instrument;

use super::super::error::{DbFieldExt, SqlxResultExt};

#[derive(Clone)]
pub struct PgJobLogRepository {
    pool: PgPool,
}

impl PgJobLogRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl JobLogRepository for PgJobLogRepository {
    #[instrument(skip_all, fields(lines = logs.len()))]
    async fn create_many(&self, logs: &[JobLog]) -> DomainResult<()> {
        queries::create_many(&self.pool, logs).await
    }

    #[instrument(skip_all, fields(job_id = %job_id))]
    async fn list_by_job(
        &self,
        job_id: &JobId,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<JobLog>> {
        let params = pagination.copied().unwrap_or_default();
        let total = queries::count_by_job(&self.pool, job_id, None).await?;
        let items = queries::list_page(&self.pool, job_id, None, &params).await?;
        Ok(PaginatedResult::new(items, &params, total))
    }

    #[instrument(skip_all, fields(job_id = %job_id, node_id = %node_id))]
    async fn list_by_job_and_node(
        &self,
        job_id: &JobId,
        node_id: &NodeId,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<JobLog>> {
        let params = pagination.copied().unwrap_or_default();
        let total = queries::count_by_job(&self.pool, job_id, Some(node_id)).await?;
        let items = queries::list_page(&self.pool, job_id, Some(node_id), &params).await?;
        Ok(PaginatedResult::new(items, &params, total))
    }

    #[instrument(skip_all, fields(job_id = %job_id))]
    async fn list_all_by_job(
        &self,
        job_id: &JobId,
        node_id: Option<&NodeId>,
    ) -> DomainResult<Vec<JobLog>> {
        queries::list_all(&self.pool, job_id, node_id).await
    }
}

#[allow(clippy::wildcard_imports)]
pub mod queries {
    use super::*;

    fn row_into_log(
        id: String,
        job_id: String,
        node_id: String,
        stream: String,
        line: String,
        timestamp: DateTime<Utc>,
        created_at: DateTime<Utc>,
    ) -> DomainResult<JobLog> {
        let node_id = NodeId::new(node_id).db_field("node_id")?;
        let stream = LogStream::new(stream).db_field("log stream")?;
        Ok(JobLog::from_persistence(
            JobLogId::new(id),
            JobId::new(job_id),
            node_id,
            stream,
            line,
            timestamp,
            created_at,
        ))
    }

    /// One statement for the whole batch.
    pub async fn create_many<'e, E>(executor: E, logs: &[JobLog]) -> DomainResult<()>
    where
        E: PgExecutor<'e>,
    {
        let column = |f: fn(&JobLog) -> &str| logs.iter().map(f).map(str::to_owned).collect();
        let ids: Vec<String> = column(|l| l.id().as_str());
        let jobs: Vec<String> = column(|l| l.job_id().as_str());
        let nodes: Vec<String> = column(|l| l.node_id().as_str());
        let streams: Vec<String> = column(|l| l.stream().as_str());
        let lines: Vec<String> = column(JobLog::line);
        let timestamps: Vec<DateTime<Utc>> = logs.iter().map(JobLog::timestamp).collect();
        let created: Vec<DateTime<Utc>> = logs.iter().map(JobLog::created_at).collect();
        sqlx::query!(
            r#"
            INSERT INTO job_logs (id, job_id, node_id, stream, line, timestamp, created_at)
            SELECT * FROM UNNEST($1::text[], $2::text[], $3::text[], $4::text[], $5::text[], $6::timestamptz[], $7::timestamptz[])
            "#,
            &ids,
            &jobs,
            &nodes,
            &streams,
            &lines,
            &timestamps,
            &created,
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(())
    }

    pub async fn count_by_job<'e, E>(
        executor: E,
        job_id: &JobId,
        node_id: Option<&NodeId>,
    ) -> DomainResult<u64>
    where
        E: PgExecutor<'e>,
    {
        let row = sqlx::query!(
            r#"
            SELECT COUNT(*) AS "count!"
            FROM job_logs
            WHERE job_id = $1
              AND ($2::text IS NULL OR node_id = $2)
            "#,
            job_id.as_str(),
            node_id.map(AsRef::as_ref),
        )
        .fetch_one(executor)
        .await
        .to_domain()?;
        Ok(u64::try_from(row.count).unwrap_or(0))
    }

    pub async fn list_page<'e, E>(
        executor: E,
        job_id: &JobId,
        node_id: Option<&NodeId>,
        params: &PaginationParams,
    ) -> DomainResult<Vec<JobLog>>
    where
        E: PgExecutor<'e>,
    {
        let limit = i64::try_from(params.limit()).unwrap_or(i64::MAX);
        let offset = i64::try_from(params.offset()).unwrap_or(i64::MAX);
        let rows = sqlx::query!(
            r#"
            SELECT id, job_id, node_id, stream, line, timestamp, created_at
            FROM job_logs
            WHERE job_id = $1
              AND ($2::text IS NULL OR node_id = $2)
            ORDER BY timestamp ASC, id ASC
            LIMIT $3 OFFSET $4
            "#,
            job_id.as_str(),
            node_id.map(AsRef::as_ref),
            limit,
            offset,
        )
        .fetch_all(executor)
        .await
        .to_domain()?;
        rows.into_iter()
            .map(|r| {
                row_into_log(
                    r.id,
                    r.job_id,
                    r.node_id,
                    r.stream,
                    r.line,
                    r.timestamp,
                    r.created_at,
                )
            })
            .collect()
    }

    pub async fn list_all<'e, E>(
        executor: E,
        job_id: &JobId,
        node_id: Option<&NodeId>,
    ) -> DomainResult<Vec<JobLog>>
    where
        E: PgExecutor<'e>,
    {
        let rows = sqlx::query!(
            r#"
            SELECT id, job_id, node_id, stream, line, timestamp, created_at
            FROM job_logs
            WHERE job_id = $1
              AND ($2::text IS NULL OR node_id = $2)
            ORDER BY timestamp ASC, id ASC
            "#,
            job_id.as_str(),
            node_id.map(AsRef::as_ref),
        )
        .fetch_all(executor)
        .await
        .to_domain()?;
        rows.into_iter()
            .map(|r| {
                row_into_log(
                    r.id,
                    r.job_id,
                    r.node_id,
                    r.stream,
                    r.line,
                    r.timestamp,
                    r.created_at,
                )
            })
            .collect()
    }
}
