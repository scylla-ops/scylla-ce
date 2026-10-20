use crate::domain::app::AppCredential;
use crate::domain::app::{AppSecretHash, AppSecretLabel};
use crate::domain::errors::DomainResult;
use crate::domain::ids::{AppCredentialId, AppId};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scylla_core::application::app::AppCredentialRepository;
use sqlx::{PgExecutor, PgPool};
use tracing::instrument;

use super::super::error::{DbFieldExt, SqlxResultExt};

pub async fn insert<'e, E>(executor: E, credential: &AppCredential) -> DomainResult<()>
where
    E: PgExecutor<'e>,
{
    sqlx::query!(
        r#"
        INSERT INTO app_secrets (id, app_id, label, secret_hash, enabled, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
        credential.id().as_str(),
        credential.app_id().as_str(),
        credential.label().as_str(),
        credential.secret_hash().as_str(),
        credential.is_enabled(),
        credential.created_at(),
        credential.updated_at(),
    )
    .execute(executor)
    .await
    .to_domain()?;
    Ok(())
}

#[derive(Clone)]
pub struct PgAppCredentialRepository {
    pool: PgPool,
}

impl PgAppCredentialRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AppCredentialRepository for PgAppCredentialRepository {
    #[instrument(skip_all, fields(secret_id = %credential.id(), app_id = %credential.app_id()))]
    async fn create(&self, credential: &AppCredential) -> DomainResult<()> {
        insert(&self.pool, credential).await
    }

    #[instrument(skip_all, fields(secret_id = %id))]
    async fn find_by_id(&self, id: &AppCredentialId) -> DomainResult<AppCredential> {
        let rec = sqlx::query!(
            r#"
            SELECT id, app_id, label, secret_hash, enabled, created_at, updated_at
            FROM app_secrets
            WHERE id = $1
            "#,
            id.as_str(),
        )
        .fetch_one(&self.pool)
        .await
        .not_found_as("AppCredential", id)?;
        row_into_credential(
            rec.id,
            rec.app_id,
            rec.label,
            rec.secret_hash,
            rec.enabled,
            rec.created_at,
            rec.updated_at,
        )
    }

    #[instrument(skip_all, fields(app_id = %app_id))]
    async fn list_by_app(&self, app_id: &AppId) -> DomainResult<Vec<AppCredential>> {
        let rows = sqlx::query!(
            r#"
            SELECT id, app_id, label, secret_hash, enabled, created_at, updated_at
            FROM app_secrets
            WHERE app_id = $1
            ORDER BY created_at DESC
            "#,
            app_id.as_str(),
        )
        .fetch_all(&self.pool)
        .await
        .to_domain()?;
        rows.into_iter()
            .map(|r| {
                row_into_credential(
                    r.id,
                    r.app_id,
                    r.label,
                    r.secret_hash,
                    r.enabled,
                    r.created_at,
                    r.updated_at,
                )
            })
            .collect()
    }

    #[instrument(skip_all, fields(app_id = %app_id))]
    async fn list_enabled_by_app(&self, app_id: &AppId) -> DomainResult<Vec<AppCredential>> {
        let rows = sqlx::query!(
            r#"
            SELECT id, app_id, label, secret_hash, enabled, created_at, updated_at
            FROM app_secrets
            WHERE app_id = $1 AND enabled = TRUE
            ORDER BY created_at DESC
            "#,
            app_id.as_str(),
        )
        .fetch_all(&self.pool)
        .await
        .to_domain()?;
        rows.into_iter()
            .map(|r| {
                row_into_credential(
                    r.id,
                    r.app_id,
                    r.label,
                    r.secret_hash,
                    r.enabled,
                    r.created_at,
                    r.updated_at,
                )
            })
            .collect()
    }

    #[instrument(skip_all, fields(secret_id = %id, enabled))]
    async fn set_enabled(&self, id: &AppCredentialId, enabled: bool) -> DomainResult<()> {
        sqlx::query!(
            "UPDATE app_secrets SET enabled = $2, updated_at = NOW() WHERE id = $1",
            id.as_str(),
            enabled,
        )
        .execute(&self.pool)
        .await
        .to_domain()?;
        Ok(())
    }

    #[instrument(skip_all, fields(secret_id = %id))]
    async fn set_label(&self, id: &AppCredentialId, label: &AppSecretLabel) -> DomainResult<()> {
        sqlx::query!(
            "UPDATE app_secrets SET label = $2, updated_at = NOW() WHERE id = $1",
            id.as_str(),
            label.as_str(),
        )
        .execute(&self.pool)
        .await
        .to_domain()?;
        Ok(())
    }

    #[instrument(skip_all, fields(secret_id = %id))]
    async fn delete(&self, id: &AppCredentialId) -> DomainResult<()> {
        sqlx::query!("DELETE FROM app_secrets WHERE id = $1", id.as_str())
            .execute(&self.pool)
            .await
            .to_domain()?;
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn row_into_credential(
    id: String,
    app_id: String,
    label: String,
    secret_hash: String,
    enabled: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
) -> DomainResult<AppCredential> {
    let label = AppSecretLabel::new(label).db_field("app secret label")?;
    let secret_hash = AppSecretHash::new(secret_hash).db_field("app secret_hash")?;
    Ok(AppCredential::from_persistence(
        AppCredentialId::new(id),
        AppId::new(app_id),
        label,
        secret_hash,
        enabled,
        created_at,
        updated_at,
    ))
}
