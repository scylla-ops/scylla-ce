use crate::domain::app::AppToken;
use crate::domain::errors::DomainResult;
use crate::domain::ids::{AppCredentialId, AppId, AppTokenId};
use async_trait::async_trait;
use scylla_core::application::app::AppTokenRepository;
use sqlx::{PgExecutor, PgPool};
use tracing::instrument;

use super::super::error::SqlxResultExt;

#[derive(Clone)]
pub struct PgAppTokenRepository {
    pool: PgPool,
}

impl PgAppTokenRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AppTokenRepository for PgAppTokenRepository {
    #[instrument(skip_all, fields(token_id = %token.id()))]
    async fn create(&self, token: &AppToken) -> DomainResult<()> {
        queries::create(&self.pool, token).await
    }

    #[instrument(skip(self, token))]
    async fn find_by_token(&self, token: &str) -> DomainResult<AppToken> {
        queries::find_by_token(&self.pool, token).await
    }
}

#[allow(clippy::wildcard_imports)]
pub mod queries {
    use super::*;

    /// Stores the SHA-256 of the token, never the token.
    pub async fn create<'e, E>(executor: E, token: &AppToken) -> DomainResult<()>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query!(
            r#"
            INSERT INTO app_tokens (id, token_hash, app_id, secret_id, created_at, expires_at)
            VALUES ($1, encode(sha256(convert_to($2, 'UTF8')), 'hex'), $3, $4, $5, $6)
            "#,
            token.id().as_str(),
            token.token(),
            token.app_id().as_str(),
            token.secret_id().as_str(),
            token.created_at(),
            token.expires_at(),
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(())
    }

    /// The join filters on the secret being enabled and the app active, so a change applies on the next request.
    pub async fn find_by_token<'e, E>(executor: E, token: &str) -> DomainResult<AppToken>
    where
        E: PgExecutor<'e>,
    {
        let rec = sqlx::query!(
            r#"
            SELECT t.id, t.app_id, t.secret_id, t.created_at, t.expires_at
            FROM app_tokens t
            JOIN app_secrets s ON s.id = t.secret_id AND s.enabled = TRUE
            JOIN apps a ON a.id = t.app_id AND a.is_active = TRUE
            WHERE t.token_hash = encode(sha256(convert_to($1, 'UTF8')), 'hex')
            "#,
            token,
        )
        .fetch_one(executor)
        .await
        // Never echo the bearer token into the error id.
        .not_found_as("AppToken", "<token>")?;
        Ok(AppToken::from_persistence(
            AppTokenId::new(rec.id),
            token.to_owned(),
            AppId::new(rec.app_id),
            AppCredentialId::new(rec.secret_id),
            rec.created_at,
            rec.expires_at,
        ))
    }
}
