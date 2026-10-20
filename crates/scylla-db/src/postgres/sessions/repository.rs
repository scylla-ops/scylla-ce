use crate::domain::errors::DomainResult;
use crate::domain::ids::{SessionId, UserId};
use crate::domain::session::{ACTIVITY_INTERVAL, Session, SessionClient};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scylla_core::application::SessionRepository;
use sqlx::{PgExecutor, PgPool};
use tracing::instrument;

use super::super::error::SqlxResultExt;

#[derive(Clone)]
pub struct PgSessionRepository {
    pool: PgPool,
}

impl PgSessionRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SessionRepository for PgSessionRepository {
    #[instrument(skip_all, fields(session_id = %session.id()))]
    async fn create(&self, session: &Session) -> DomainResult<Session> {
        queries::create(&self.pool, session).await
    }

    #[instrument(skip(self, token))]
    async fn find_by_token(&self, token: &str) -> DomainResult<Session> {
        queries::find_by_token(&self.pool, token).await
    }

    #[instrument(skip(self, token))]
    async fn delete_by_token(&self, token: &str) -> DomainResult<()> {
        queries::delete_by_token(&self.pool, token).await
    }

    #[instrument(skip(self))]
    async fn delete_expired(&self) -> DomainResult<u64> {
        queries::delete_expired(&self.pool).await
    }

    #[instrument(skip(self))]
    async fn touch(&self, id: &SessionId, at: DateTime<Utc>) -> DomainResult<bool> {
        queries::touch(&self.pool, id, at).await
    }
}

#[allow(clippy::wildcard_imports)]
pub mod queries {
    use super::*;

    /// Stores the SHA-256 of the token, never the token.
    pub async fn create<'e, E>(executor: E, session: &Session) -> DomainResult<Session>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query!(
            r#"
            INSERT INTO sessions (
                id, token_hash, user_id, created_at, expires_at, last_active_at,
                user_agent, ip_address
            )
            VALUES ($1, encode(sha256(convert_to($2, 'UTF8')), 'hex'), $3, $4, $5, $6, $7, $8)
            "#,
            session.id().as_str(),
            session.token(),
            session.user_id().as_str(),
            session.created_at(),
            session.expires_at(),
            session.last_active_at(),
            session.client().user_agent(),
            session.client().ip_address().map(|ip| ip.to_string()),
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(session.clone())
    }

    pub async fn find_by_token<'e, E>(executor: E, token: &str) -> DomainResult<Session>
    where
        E: PgExecutor<'e>,
    {
        let rec = sqlx::query!(
            r#"
            SELECT id, user_id, created_at, expires_at, last_active_at, user_agent, ip_address
            FROM sessions
            WHERE token_hash = encode(sha256(convert_to($1, 'UTF8')), 'hex')
            "#,
            token,
        )
        .fetch_one(executor)
        .await
        // Never echo the session token into the error id.
        .not_found_as("Session", "<token>")?;
        Ok(Session::from_persistence(
            SessionId::new(rec.id),
            token.to_owned(),
            UserId::new(rec.user_id),
            rec.created_at,
            rec.expires_at,
            rec.last_active_at,
        )
        .with_client(SessionClient::new(
            rec.user_agent.as_deref(),
            rec.ip_address.as_deref(),
        )))
    }

    pub async fn delete_by_token<'e, E>(executor: E, token: &str) -> DomainResult<()>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query!(
            "DELETE FROM sessions WHERE token_hash = encode(sha256(convert_to($1, 'UTF8')), 'hex')",
            token,
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(())
    }

    pub async fn delete_expired<'e, E>(executor: E) -> DomainResult<u64>
    where
        E: PgExecutor<'e>,
    {
        let res = sqlx::query!("DELETE FROM sessions WHERE expires_at <= NOW()")
            .execute(executor)
            .await
            .to_domain()?;
        Ok(res.rows_affected())
    }

    /// One conditional write: two calls in the same interval move the activity once.
    pub async fn touch<'e, E>(executor: E, id: &SessionId, at: DateTime<Utc>) -> DomainResult<bool>
    where
        E: PgExecutor<'e>,
    {
        let res = sqlx::query!(
            "UPDATE sessions SET last_active_at = $2 WHERE id = $1 AND last_active_at <= $3",
            id.as_str(),
            at,
            at - ACTIVITY_INTERVAL,
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(res.rows_affected() > 0)
    }
}
