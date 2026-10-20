use crate::domain::errors::DomainResult;
use crate::domain::ids::TriggerId;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scylla_core::application::TriggerDeliveryRepository;
use sqlx::{PgExecutor, PgPool};
use tracing::instrument;

use super::super::error::SqlxResultExt;

#[derive(Clone)]
pub struct PgTriggerDeliveryRepository {
    pool: PgPool,
}

impl PgTriggerDeliveryRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl TriggerDeliveryRepository for PgTriggerDeliveryRepository {
    #[instrument(skip_all, fields(trigger_id = %trigger_id, delivery_id))]
    async fn record_or_detect(
        &self,
        trigger_id: &TriggerId,
        delivery_id: &str,
        received_at: DateTime<Utc>,
    ) -> DomainResult<bool> {
        queries::record_or_detect(&self.pool, trigger_id, delivery_id, received_at).await
    }

    #[instrument(skip_all, fields(trigger_id = %trigger_id, delivery_id))]
    async fn forget(&self, trigger_id: &TriggerId, delivery_id: &str) -> DomainResult<()> {
        queries::forget(&self.pool, trigger_id, delivery_id).await
    }
}

#[allow(clippy::wildcard_imports)]
pub mod queries {
    use super::*;

    pub async fn record_or_detect<'e, E>(
        executor: E,
        trigger_id: &TriggerId,
        delivery_id: &str,
        received_at: DateTime<Utc>,
    ) -> DomainResult<bool>
    where
        E: PgExecutor<'e>,
    {
        let res = sqlx::query!(
            r#"
            INSERT INTO trigger_deliveries (trigger_id, delivery_id, received_at)
            VALUES ($1, $2, $3)
            ON CONFLICT (trigger_id, delivery_id) DO NOTHING
            "#,
            trigger_id.as_str(),
            delivery_id,
            received_at,
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn forget<'e, E>(
        executor: E,
        trigger_id: &TriggerId,
        delivery_id: &str,
    ) -> DomainResult<()>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query!(
            "DELETE FROM trigger_deliveries WHERE trigger_id = $1 AND delivery_id = $2",
            trigger_id.as_str(),
            delivery_id,
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(())
    }
}
