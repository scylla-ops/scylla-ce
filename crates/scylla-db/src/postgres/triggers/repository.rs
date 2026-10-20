use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{PipelineId, TriggerId};
use crate::domain::trigger::{FireObservation, Trigger};
use crate::domain::trigger::{TriggerInput, TriggerName, TriggerSource};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scylla_core::application::{NextFire, TriggerRepository};
use sqlx::{PgConnection, PgExecutor, PgPool, types::Json};
use tracing::instrument;

use super::super::error::{DbFieldExt, SqlxResultExt};
use super::super::version::{from_db, to_db, written};

#[derive(Clone)]
pub struct PgTriggerRepository {
    pool: PgPool,
}

impl PgTriggerRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl TriggerRepository for PgTriggerRepository {
    #[instrument(skip_all, fields(trigger_id = %trigger.id()))]
    async fn create(
        &self,
        trigger: &Trigger,
        webhook_secret_enc: Option<&[u8]>,
    ) -> DomainResult<Trigger> {
        queries::create(&self.pool, trigger, webhook_secret_enc).await
    }

    #[instrument(skip_all, fields(trigger_id = %id))]
    async fn find_by_id(&self, id: &TriggerId) -> DomainResult<Trigger> {
        queries::find_by_id(&self.pool, id).await
    }

    #[instrument(skip_all, fields(trigger_id = %id))]
    async fn webhook_secret(&self, id: &TriggerId) -> DomainResult<Option<Vec<u8>>> {
        queries::webhook_secret(&self.pool, id).await
    }

    #[instrument(skip_all, fields(trigger_id = %trigger.id(), version = trigger.version()))]
    async fn update(&self, trigger: &Trigger) -> DomainResult<Trigger> {
        let updated = queries::update(&self.pool, trigger).await?;
        written(
            updated,
            "Trigger",
            trigger.id(),
            queries::find_by_id(&self.pool, trigger.id()),
        )
        .await
    }

    #[instrument(skip_all, fields(trigger_id = %trigger.id(), version = trigger.version()))]
    async fn delete(&self, trigger: &Trigger) -> DomainResult<()> {
        let deleted = queries::delete(&self.pool, trigger).await?.then_some(());
        written(
            deleted,
            "Trigger",
            trigger.id(),
            queries::find_by_id(&self.pool, trigger.id()),
        )
        .await
    }

    #[instrument(skip_all, fields(pipeline_id = %pipeline_id))]
    async fn list_by_pipeline(&self, pipeline_id: &PipelineId) -> DomainResult<Vec<Trigger>> {
        queries::list_by_pipeline(&self.pool, pipeline_id).await
    }

    #[instrument(skip_all, fields(trigger_id = %id))]
    async fn record_fire(&self, id: &TriggerId, observation: &FireObservation) -> DomainResult<()> {
        queries::record_fire(&self.pool, id, observation).await
    }

    #[instrument(skip_all)]
    async fn seed_cron(&self, compute_next: &NextFire<'_>) -> DomainResult<Vec<Trigger>> {
        let mut tx = self.pool.begin().await.to_domain()?;
        let unscheduled = queries::lock_unscheduled_cron(&mut *tx).await?;
        let seeded = advance(&mut tx, unscheduled, compute_next).await?;
        tx.commit().await.to_domain()?;
        Ok(seeded)
    }

    #[instrument(skip_all, fields(now = %now, limit))]
    async fn claim_due_cron(
        &self,
        now: DateTime<Utc>,
        limit: i64,
        compute_next: &NextFire<'_>,
    ) -> DomainResult<Vec<Trigger>> {
        let mut tx = self.pool.begin().await.to_domain()?;
        let due = queries::lock_due_cron(&mut *tx, now, limit).await?;
        let claimed = advance(&mut tx, due, compute_next).await?;
        tx.commit().await.to_domain()?;
        Ok(claimed)
    }
}

/// Writes each locked row's next fire time in the caller's transaction, so an occurrence is
/// consumed exactly once; a row whose next time cannot be computed is left and excluded.
async fn advance(
    tx: &mut PgConnection,
    locked: Vec<Trigger>,
    compute_next: &NextFire<'_>,
) -> DomainResult<Vec<Trigger>> {
    let mut advanced = Vec::with_capacity(locked.len());
    for trigger in locked {
        let Ok(next) = compute_next(&trigger) else {
            continue;
        };
        queries::set_next_fire_at(&mut *tx, trigger.id(), next).await?;
        advanced.push(trigger);
    }
    Ok(advanced)
}

/// `kind` is written for indexing but never read back; the source kind comes from the JSONB tag.
#[derive(sqlx::FromRow)]
struct TriggerRow {
    id: String,
    pipeline_id: String,
    name: String,
    source: Json<TriggerSource>,
    inputs: Json<Vec<TriggerInput>>,
    enabled: bool,
    next_fire_at: Option<DateTime<Utc>>,
    last_fired_at: Option<DateTime<Utc>>,
    last_status: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    version: i64,
}

impl TryFrom<TriggerRow> for Trigger {
    type Error = DomainError;
    fn try_from(r: TriggerRow) -> DomainResult<Self> {
        let name = TriggerName::new(r.name).db_field("trigger name")?;
        Ok(Trigger::from_persistence(
            TriggerId::new(r.id),
            PipelineId::new(r.pipeline_id),
            name,
            r.source.0,
            r.inputs.0,
            r.enabled,
            r.next_fire_at,
            r.last_fired_at,
            r.last_status,
            r.created_at,
            r.updated_at,
            from_db(r.version),
        ))
    }
}

#[allow(clippy::wildcard_imports)]
pub mod queries {
    use super::*;

    pub async fn create<'e, E>(
        executor: E,
        trigger: &Trigger,
        webhook_secret_enc: Option<&[u8]>,
    ) -> DomainResult<Trigger>
    where
        E: PgExecutor<'e>,
    {
        let source = Json(trigger.source().clone());
        let inputs = Json(trigger.inputs().to_vec());
        sqlx::query!(
            r#"
            INSERT INTO pipeline_triggers
                (id, pipeline_id, name, kind, source, inputs, enabled,
                 next_fire_at, last_fired_at, last_status, webhook_secret_enc,
                 created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            "#,
            trigger.id().as_str(),
            trigger.pipeline_id().as_str(),
            trigger.name().as_str(),
            trigger.source().kind().as_str(),
            source as _,
            inputs as _,
            trigger.is_enabled(),
            trigger.next_fire_at(),
            trigger.last_fired_at(),
            trigger.last_status(),
            webhook_secret_enc,
            trigger.created_at(),
            trigger.updated_at(),
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(trigger.clone())
    }

    pub async fn webhook_secret<'e, E>(executor: E, id: &TriggerId) -> DomainResult<Option<Vec<u8>>>
    where
        E: PgExecutor<'e>,
    {
        let row = sqlx::query!(
            r#"SELECT webhook_secret_enc FROM pipeline_triggers WHERE id = $1"#,
            id.as_str(),
        )
        .fetch_optional(executor)
        .await
        .to_domain()?;
        Ok(row.and_then(|r| r.webhook_secret_enc))
    }

    pub async fn find_by_id<'e, E>(executor: E, id: &TriggerId) -> DomainResult<Trigger>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query_as!(
            TriggerRow,
            r#"
            SELECT id, pipeline_id, name,
                   source AS "source: Json<TriggerSource>",
                   inputs AS "inputs: Json<Vec<TriggerInput>>",
                   enabled, next_fire_at, last_fired_at, last_status, created_at, updated_at,
                   version
            FROM pipeline_triggers
            WHERE id = $1
            "#,
            id.as_str(),
        )
        .fetch_one(executor)
        .await
        .not_found_as("Trigger", id)?
        .try_into()
    }

    pub async fn update<'e, E>(executor: E, trigger: &Trigger) -> DomainResult<Option<Trigger>>
    where
        E: PgExecutor<'e>,
    {
        let source = Json(trigger.source().clone());
        let inputs = Json(trigger.inputs().to_vec());
        sqlx::query_as!(
            TriggerRow,
            r#"
            UPDATE pipeline_triggers
            SET name = $2,
                source = $3,
                inputs = $4,
                enabled = $5,
                next_fire_at = $6,
                updated_at = $7,
                version = version + 1
            WHERE id = $1 AND version = $8
            RETURNING id, pipeline_id, name,
                      source AS "source: Json<TriggerSource>",
                      inputs AS "inputs: Json<Vec<TriggerInput>>",
                      enabled, next_fire_at, last_fired_at, last_status, created_at, updated_at,
                      version
            "#,
            trigger.id().as_str(),
            trigger.name().as_str(),
            source as _,
            inputs as _,
            trigger.is_enabled(),
            trigger.next_fire_at(),
            trigger.updated_at(),
            to_db(trigger.version()),
        )
        .fetch_optional(executor)
        .await
        .to_domain()?
        .map(Trigger::try_from)
        .transpose()
    }

    pub async fn delete<'e, E>(executor: E, trigger: &Trigger) -> DomainResult<bool>
    where
        E: PgExecutor<'e>,
    {
        let res = sqlx::query!(
            "DELETE FROM pipeline_triggers WHERE id = $1 AND version = $2",
            trigger.id().as_str(),
            to_db(trigger.version()),
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn record_fire<'e, E>(
        executor: E,
        id: &TriggerId,
        observation: &FireObservation,
    ) -> DomainResult<()>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query!(
            "UPDATE pipeline_triggers SET last_fired_at = $2, last_status = $3 WHERE id = $1",
            id.as_str(),
            observation.fired_at,
            observation.status,
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(())
    }

    pub async fn list_by_pipeline<'e, E>(
        executor: E,
        pipeline_id: &PipelineId,
    ) -> DomainResult<Vec<Trigger>>
    where
        E: PgExecutor<'e>,
    {
        let rows: Vec<TriggerRow> = sqlx::query_as!(
            TriggerRow,
            r#"
            SELECT id, pipeline_id, name,
                   source AS "source: Json<TriggerSource>",
                   inputs AS "inputs: Json<Vec<TriggerInput>>",
                   enabled, next_fire_at, last_fired_at, last_status, created_at, updated_at,
                   version
            FROM pipeline_triggers
            WHERE pipeline_id = $1
            ORDER BY created_at
            "#,
            pipeline_id.as_str(),
        )
        .fetch_all(executor)
        .await
        .to_domain()?;
        rows.into_iter().map(Trigger::try_from).collect()
    }

    pub async fn lock_unscheduled_cron<'e, E>(executor: E) -> DomainResult<Vec<Trigger>>
    where
        E: PgExecutor<'e>,
    {
        let rows: Vec<TriggerRow> = sqlx::query_as!(
            TriggerRow,
            r#"
            SELECT id, pipeline_id, name,
                   source AS "source: Json<TriggerSource>",
                   inputs AS "inputs: Json<Vec<TriggerInput>>",
                   enabled, next_fire_at, last_fired_at, last_status, created_at, updated_at,
                   version
            FROM pipeline_triggers
            WHERE enabled AND kind = 'cron' AND next_fire_at IS NULL
            ORDER BY created_at
            FOR UPDATE SKIP LOCKED
            "#,
        )
        .fetch_all(executor)
        .await
        .to_domain()?;
        rows.into_iter().map(Trigger::try_from).collect()
    }

    pub async fn lock_due_cron<'e, E>(
        executor: E,
        now: DateTime<Utc>,
        limit: i64,
    ) -> DomainResult<Vec<Trigger>>
    where
        E: PgExecutor<'e>,
    {
        let rows: Vec<TriggerRow> = sqlx::query_as!(
            TriggerRow,
            r#"
            SELECT id, pipeline_id, name,
                   source AS "source: Json<TriggerSource>",
                   inputs AS "inputs: Json<Vec<TriggerInput>>",
                   enabled, next_fire_at, last_fired_at, last_status, created_at, updated_at,
                   version
            FROM pipeline_triggers
            WHERE enabled
              AND kind = 'cron'
              AND next_fire_at IS NOT NULL
              AND next_fire_at <= $1
            ORDER BY next_fire_at
            LIMIT $2
            FOR UPDATE SKIP LOCKED
            "#,
            now,
            limit,
        )
        .fetch_all(executor)
        .await
        .to_domain()?;
        rows.into_iter().map(Trigger::try_from).collect()
    }

    pub async fn set_next_fire_at<'e, E>(
        executor: E,
        id: &TriggerId,
        next_fire_at: DateTime<Utc>,
    ) -> DomainResult<()>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query!(
            "UPDATE pipeline_triggers SET next_fire_at = $2 WHERE id = $1",
            id.as_str(),
            next_fire_at,
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(())
    }
}
