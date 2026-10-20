use crate::domain::agent::{Agent, AgentHost};
use crate::domain::errors::DomainResult;
use crate::domain::ids::{AppId, OrganizationId};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scylla_core::application::agent::repository::{AgentRepository, AgentStats};
use sqlx::{PgExecutor, PgPool};
use tracing::instrument;

use super::super::error::SqlxResultExt;

fn host_from_columns(
    reported_at: Option<DateTime<Utc>>,
    version: Option<String>,
    os: Option<String>,
    arch: Option<String>,
    hostname: Option<String>,
    cpu_count: Option<i32>,
    total_memory_mb: Option<i64>,
) -> Option<AgentHost> {
    Some(AgentHost {
        version: version.unwrap_or_default(),
        os: os.unwrap_or_default(),
        arch: arch.unwrap_or_default(),
        hostname: hostname.unwrap_or_default(),
        cpu_count,
        total_memory_mb,
        reported_at: reported_at?,
    })
}

pub async fn insert<'e, E>(executor: E, agent: &Agent) -> DomainResult<()>
where
    E: PgExecutor<'e>,
{
    sqlx::query!(
        r#"
        INSERT INTO agents (app_id, last_seen, created_at)
        VALUES ($1, $2, $3)
        "#,
        agent.app_id().as_str(),
        agent.last_seen(),
        agent.created_at(),
    )
    .execute(executor)
    .await
    .to_domain()?;
    Ok(())
}

#[derive(Clone)]
pub struct PgAgentRepository {
    pool: PgPool,
}

impl PgAgentRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AgentRepository for PgAgentRepository {
    #[instrument(skip_all, fields(app_id = %app_id))]
    async fn find_by_app_id(&self, app_id: &AppId) -> DomainResult<Agent> {
        let rec = sqlx::query!(
            r#"
            SELECT app_id, last_seen, created_at, agent_version, host_os, host_arch,
                   hostname, cpu_count, total_memory_mb, host_reported_at
            FROM agents
            WHERE app_id = $1
            "#,
            app_id.as_str(),
        )
        .fetch_one(&self.pool)
        .await
        .not_found_as("Agent", app_id)?;
        Ok(Agent::from_persistence(
            AppId::new(rec.app_id),
            rec.last_seen,
            rec.created_at,
            host_from_columns(
                rec.host_reported_at,
                rec.agent_version,
                rec.host_os,
                rec.host_arch,
                rec.hostname,
                rec.cpu_count,
                rec.total_memory_mb,
            ),
        ))
    }

    #[instrument(skip_all, fields(org_id = %org_id))]
    async fn list_by_organization(&self, org_id: &OrganizationId) -> DomainResult<Vec<Agent>> {
        let rows = sqlx::query!(
            r#"
            SELECT w.app_id, w.last_seen, w.created_at, w.agent_version, w.host_os,
                   w.host_arch, w.hostname, w.cpu_count, w.total_memory_mb,
                   w.host_reported_at
            FROM agents w
            JOIN apps a ON a.id = w.app_id
            WHERE a.organization_id = $1
            ORDER BY w.created_at DESC
            "#,
            org_id.as_str(),
        )
        .fetch_all(&self.pool)
        .await
        .to_domain()?;
        Ok(rows
            .into_iter()
            .map(|r| {
                Agent::from_persistence(
                    AppId::new(r.app_id),
                    r.last_seen,
                    r.created_at,
                    host_from_columns(
                        r.host_reported_at,
                        r.agent_version,
                        r.host_os,
                        r.host_arch,
                        r.hostname,
                        r.cpu_count,
                        r.total_memory_mb,
                    ),
                )
            })
            .collect())
    }

    #[instrument(skip_all, fields(app_id = %app_id))]
    async fn touch_last_seen(&self, app_id: &AppId, at: DateTime<Utc>) -> DomainResult<()> {
        // Upsert: an agent without a row (pre-migration) self-heals; a deleted app is a no-op.
        sqlx::query!(
            r#"
            INSERT INTO agents (app_id, last_seen, created_at)
            SELECT id, $2, NOW() FROM apps WHERE id = $1
            ON CONFLICT (app_id) DO UPDATE SET last_seen = $2
            "#,
            app_id.as_str(),
            at,
        )
        .execute(&self.pool)
        .await
        .to_domain()?;
        Ok(())
    }

    #[instrument(skip_all, fields(app_id = %app_id, hostname = %host.hostname))]
    async fn record_host(&self, app_id: &AppId, host: &AgentHost) -> DomainResult<()> {
        sqlx::query!(
            r#"
            INSERT INTO agents (app_id, created_at, agent_version, host_os, host_arch,
                                hostname, cpu_count, total_memory_mb, host_reported_at)
            SELECT id, NOW(), $2, $3, $4, $5, $6, $7, $8 FROM apps WHERE id = $1
            ON CONFLICT (app_id) DO UPDATE SET
                agent_version    = $2,
                host_os          = $3,
                host_arch        = $4,
                hostname         = $5,
                cpu_count        = $6,
                total_memory_mb  = $7,
                host_reported_at = $8
            "#,
            app_id.as_str(),
            host.version,
            host.os,
            host.arch,
            host.hostname,
            host.cpu_count,
            host.total_memory_mb,
            host.reported_at,
        )
        .execute(&self.pool)
        .await
        .to_domain()?;
        Ok(())
    }

    #[instrument(skip_all, fields(app_id = %app_id))]
    async fn agent_stats(&self, app_id: &AppId) -> DomainResult<AgentStats> {
        let rec = sqlx::query!(
            r#"
            SELECT
                COUNT(*)                                          AS "total!",
                COUNT(*) FILTER (WHERE status = 'pending')        AS "pending!",
                COUNT(*) FILTER (WHERE status = 'running')        AS "running!",
                COUNT(*) FILTER (WHERE status = 'completed')      AS "completed!",
                COUNT(*) FILTER (WHERE status = 'failed')         AS "failed!",
                COUNT(*) FILTER (WHERE status = 'cancelled')      AS "cancelled!",
                COUNT(*) FILTER (WHERE status = 'orphaned')       AS "orphaned!",
                MAX(created_at)                                   AS "last_run_at",
                ROUND((percentile_cont(0.5) WITHIN GROUP (
                    ORDER BY EXTRACT(EPOCH FROM finished_at - started_at)::double precision * 1000
                ) FILTER (WHERE started_at IS NOT NULL AND finished_at IS NOT NULL))::numeric)::bigint
                                                                  AS "median_duration_ms?",
                ROUND((percentile_cont(0.95) WITHIN GROUP (
                    ORDER BY EXTRACT(EPOCH FROM finished_at - started_at)::double precision * 1000
                ) FILTER (WHERE started_at IS NOT NULL AND finished_at IS NOT NULL))::numeric)::bigint
                                                                  AS "p95_duration_ms?"
            FROM jobs
            WHERE agent_app_id = $1
            "#,
            app_id.as_str(),
        )
        .fetch_one(&self.pool)
        .await
        .to_domain()?;
        let daily = sqlx::query!(
            r#"
            SELECT
                date_trunc('day', created_at)                AS "day!",
                COUNT(*) FILTER (WHERE status = 'completed') AS "completed!",
                COUNT(*) FILTER (WHERE status = 'failed')    AS "failed!",
                COUNT(*) FILTER (WHERE status = 'cancelled') AS "cancelled!",
                COUNT(*) FILTER (WHERE status = 'orphaned')  AS "orphaned!",
                ROUND((percentile_cont(0.5) WITHIN GROUP (
                    ORDER BY EXTRACT(EPOCH FROM finished_at - started_at)::double precision * 1000
                ) FILTER (WHERE started_at IS NOT NULL AND finished_at IS NOT NULL))::numeric)::bigint
                                                             AS "median_duration_ms?"
            FROM jobs
            WHERE agent_app_id = $1
              AND created_at >= NOW() - INTERVAL '30 days'
            GROUP BY 1
            ORDER BY 1
            "#,
            app_id.as_str(),
        )
        .fetch_all(&self.pool)
        .await
        .to_domain()?
        .into_iter()
        .map(|r| scylla_core::application::agent::repository::DailyOutcome {
            day: r.day,
            completed: r.completed,
            failed: r.failed,
            cancelled: r.cancelled,
            orphaned: r.orphaned,
            median_duration_ms: r.median_duration_ms,
        })
        .collect();
        Ok(AgentStats {
            daily,
            total: rec.total,
            pending: rec.pending,
            running: rec.running,
            completed: rec.completed,
            failed: rec.failed,
            cancelled: rec.cancelled,
            orphaned: rec.orphaned,
            last_run_at: rec.last_run_at,
            median_duration_ms: rec.median_duration_ms,
            p95_duration_ms: rec.p95_duration_ms,
        })
    }
}
