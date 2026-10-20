use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{ProjectId, SecretId};
use crate::domain::secret::{Secret, SecretDescription, SecretName};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scylla_core::application::secret::SecretRepository;
use sqlx::PgPool;
use tracing::instrument;

use super::super::error::{DbFieldExt, SqlxResultExt};

#[derive(Clone)]
pub struct PgSecretRepository {
    pool: PgPool,
}

impl PgSecretRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct SecretRow {
    id: String,
    project_id: String,
    name: String,
    description: String,
    encrypted_value: Vec<u8>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<SecretRow> for Secret {
    type Error = DomainError;
    fn try_from(r: SecretRow) -> DomainResult<Self> {
        let name = SecretName::new(r.name).db_field("secret name")?;
        let description = SecretDescription::new(r.description).db_field("secret description")?;
        Ok(Secret::from_persistence(
            SecretId::new(r.id),
            ProjectId::new(r.project_id),
            name,
            description,
            r.encrypted_value,
            r.created_at,
            r.updated_at,
        ))
    }
}

#[async_trait]
impl SecretRepository for PgSecretRepository {
    #[instrument(skip_all, fields(secret_id = %secret.id()))]
    async fn create(&self, secret: &Secret) -> DomainResult<()> {
        sqlx::query!(
            r#"
            INSERT INTO project_secrets
                (id, project_id, name, description, encrypted_value, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
            secret.id().as_str(),
            secret.project_id().as_str(),
            secret.name().as_str(),
            secret.description().as_str(),
            secret.encrypted_value(),
            secret.created_at(),
            secret.updated_at(),
        )
        .execute(&self.pool)
        .await
        .to_domain()?;
        Ok(())
    }

    #[instrument(skip_all, fields(secret_id = %id))]
    async fn find_by_id(&self, id: &SecretId) -> DomainResult<Secret> {
        sqlx::query_as!(
            SecretRow,
            r#"
            SELECT id, project_id, name, description, encrypted_value, created_at, updated_at
            FROM project_secrets
            WHERE id = $1
            "#,
            id.as_str(),
        )
        .fetch_one(&self.pool)
        .await
        .not_found_as("Secret", id)?
        .try_into()
    }

    #[instrument(skip_all, fields(project_id = %project_id))]
    async fn list_by_project(&self, project_id: &ProjectId) -> DomainResult<Vec<Secret>> {
        let rows: Vec<SecretRow> = sqlx::query_as!(
            SecretRow,
            r#"
            SELECT id, project_id, name, description, encrypted_value, created_at, updated_at
            FROM project_secrets
            WHERE project_id = $1
            ORDER BY name ASC
            "#,
            project_id.as_str(),
        )
        .fetch_all(&self.pool)
        .await
        .to_domain()?;
        rows.into_iter().map(Secret::try_from).collect()
    }

    #[instrument(skip_all, fields(secret_id = %id))]
    async fn delete(&self, id: &SecretId) -> DomainResult<()> {
        sqlx::query!("DELETE FROM project_secrets WHERE id = $1", id.as_str())
            .execute(&self.pool)
            .await
            .to_domain()?;
        Ok(())
    }
}
