use crate::domain::clock;
use crate::domain::errors::DomainResult;
use crate::domain::ids::{PasswordResetId, SessionId, UserId};
use crate::domain::session::SessionClient;
use crate::domain::user::{PasswordReset, ResetToken, User, reset_link_invalid};
use async_trait::async_trait;
use chrono::Duration;
use scylla_auth::authz::Principal;
use scylla_core::application::{AccountRepository, PrincipalAccess, UserSession};
use sqlx::{PgExecutor, PgPool};
use tracing::instrument;

use super::super::error::SqlxResultExt;
use super::super::grants;
use super::super::users::repository::queries as users;
use super::super::version::written;

#[derive(Clone)]
pub struct PgAccountRepository {
    pool: PgPool,
}

impl PgAccountRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AccountRepository for PgAccountRepository {
    #[instrument(skip_all, fields(user_id = %user.id(), version = user.version()))]
    async fn update_signed_out(&self, user: &User, keep: Option<&SessionId>) -> DomainResult<User> {
        let mut tx = self.pool.begin().await.to_domain()?;
        let updated = users::update(&mut *tx, user).await?;
        let stored = written(
            updated,
            "User",
            user.id(),
            users::find_by_id(&mut *tx, user.id()),
        )
        .await?;
        queries::delete_sessions(&mut *tx, user.id(), keep).await?;
        queries::delete_resets(&mut *tx, user.id(), None).await?;
        tx.commit().await.to_domain()?;
        Ok(stored)
    }

    #[instrument(skip_all, fields(user_id = %user_id))]
    async fn revoke_sessions(
        &self,
        user_id: &UserId,
        keep: Option<&SessionId>,
    ) -> DomainResult<u64> {
        queries::delete_sessions(&self.pool, user_id, keep).await
    }

    #[instrument(skip_all, fields(user_id = %user_id))]
    async fn list_sessions(&self, user_id: &UserId) -> DomainResult<Vec<UserSession>> {
        queries::list_sessions(&self.pool, user_id).await
    }

    #[instrument(skip_all, fields(user_id = %user_id, session_id = %id))]
    async fn revoke_session(&self, user_id: &UserId, id: &SessionId) -> DomainResult<bool> {
        queries::delete_session(&self.pool, user_id, id).await
    }

    /// The user row is locked first, so two requests for one user pass the cooldown one at a
    /// time.
    #[instrument(skip_all, fields(user_id = %reset.user_id()))]
    async fn issue_reset(
        &self,
        reset: &PasswordReset,
        cooldown: Option<Duration>,
    ) -> DomainResult<bool> {
        let mut tx = self.pool.begin().await.to_domain()?;
        sqlx::query_scalar!(
            "SELECT id FROM users WHERE id = $1 FOR UPDATE",
            reset.user_id().as_str(),
        )
        .fetch_one(&mut *tx)
        .await
        .not_found_as("User", reset.user_id())?;
        if let Some(cooldown) = cooldown
            && queries::made_since(&mut *tx, reset.user_id(), reset.created_at() - cooldown).await?
        {
            return Ok(false);
        }
        queries::delete_resets(&mut *tx, reset.user_id(), None).await?;
        queries::insert_reset(&mut *tx, reset).await?;
        tx.commit().await.to_domain()?;
        Ok(true)
    }

    #[instrument(skip_all)]
    async fn find_reset(&self, token: &ResetToken) -> DomainResult<PasswordReset> {
        queries::find_reset(&self.pool, token).await
    }

    #[instrument(skip_all, fields(user_id = %user.id(), version = user.version()))]
    async fn redeem_reset(&self, reset: &PasswordReset, user: &User) -> DomainResult<User> {
        let mut tx = self.pool.begin().await.to_domain()?;
        if !queries::mark_used(&mut *tx, reset.id()).await? {
            return Err(reset_link_invalid());
        }
        let updated = users::update(&mut *tx, user).await?;
        let stored = written(
            updated,
            "User",
            user.id(),
            users::find_by_id(&mut *tx, user.id()),
        )
        .await?;
        queries::delete_resets(&mut *tx, user.id(), Some(reset.id())).await?;
        queries::delete_sessions(&mut *tx, user.id(), None).await?;
        tx.commit().await.to_domain()?;
        Ok(stored)
    }

    #[instrument(skip_all, fields(user_id = %user_id))]
    async fn list_access(&self, user_id: &UserId) -> DomainResult<Vec<PrincipalAccess>> {
        grants::list_access(&self.pool, &Principal::User(user_id.clone())).await
    }
}

#[allow(clippy::wildcard_imports)]
pub mod queries {
    use super::*;

    pub async fn delete_sessions<'e, E>(
        executor: E,
        user_id: &UserId,
        keep: Option<&SessionId>,
    ) -> DomainResult<u64>
    where
        E: PgExecutor<'e>,
    {
        let res = sqlx::query!(
            "DELETE FROM sessions WHERE user_id = $1 AND ($2::text IS NULL OR id <> $2)",
            user_id.as_str(),
            keep.map(SessionId::as_str),
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(res.rows_affected())
    }

    pub async fn list_sessions<'e, E>(
        executor: E,
        user_id: &UserId,
    ) -> DomainResult<Vec<UserSession>>
    where
        E: PgExecutor<'e>,
    {
        let rows = sqlx::query!(
            r#"
            SELECT id, created_at, last_active_at, expires_at, user_agent, ip_address
            FROM sessions
            WHERE user_id = $1 AND expires_at >= $2
            ORDER BY last_active_at DESC, created_at DESC, id DESC
            "#,
            user_id.as_str(),
            clock::now(),
        )
        .fetch_all(executor)
        .await
        .to_domain()?;
        Ok(rows
            .into_iter()
            .map(|r| UserSession {
                id: SessionId::new(r.id),
                created_at: r.created_at,
                last_active_at: r.last_active_at,
                expires_at: r.expires_at,
                client: SessionClient::new(r.user_agent.as_deref(), r.ip_address.as_deref()),
                current: false,
            })
            .collect())
    }

    /// `false` when the session is not a session of the user that has not expired.
    pub async fn delete_session<'e, E>(
        executor: E,
        user_id: &UserId,
        id: &SessionId,
    ) -> DomainResult<bool>
    where
        E: PgExecutor<'e>,
    {
        let res = sqlx::query!(
            "DELETE FROM sessions WHERE id = $1 AND user_id = $2 AND expires_at >= $3",
            id.as_str(),
            user_id.as_str(),
            clock::now(),
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn delete_resets<'e, E>(
        executor: E,
        user_id: &UserId,
        keep: Option<&PasswordResetId>,
    ) -> DomainResult<u64>
    where
        E: PgExecutor<'e>,
    {
        let res = sqlx::query!(
            "DELETE FROM password_resets WHERE user_id = $1 AND ($2::text IS NULL OR id <> $2)",
            user_id.as_str(),
            keep.map(PasswordResetId::as_str),
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(res.rows_affected())
    }

    pub async fn made_since<'e, E>(
        executor: E,
        user_id: &UserId,
        since: chrono::DateTime<chrono::Utc>,
    ) -> DomainResult<bool>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query_scalar!(
            r#"SELECT EXISTS (
                SELECT 1 FROM password_resets WHERE user_id = $1 AND created_at > $2
            ) AS "made!""#,
            user_id.as_str(),
            since,
        )
        .fetch_one(executor)
        .await
        .to_domain()
    }

    /// Stores the SHA-256 of the token, never the token.
    pub async fn insert_reset<'e, E>(executor: E, reset: &PasswordReset) -> DomainResult<()>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query!(
            r#"
            INSERT INTO password_resets (id, user_id, token_hash, created_at, expires_at, used_at)
            VALUES ($1, $2, encode(sha256(convert_to($3, 'UTF8')), 'hex'), $4, $5, $6)
            "#,
            reset.id().as_str(),
            reset.user_id().as_str(),
            reset.token().as_str(),
            reset.created_at(),
            reset.expires_at(),
            reset.used_at(),
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(())
    }

    pub async fn find_reset<'e, E>(executor: E, token: &ResetToken) -> DomainResult<PasswordReset>
    where
        E: PgExecutor<'e>,
    {
        let rec = sqlx::query!(
            r#"
            SELECT id, user_id, created_at, expires_at, used_at
            FROM password_resets
            WHERE token_hash = encode(sha256(convert_to($1, 'UTF8')), 'hex')
            "#,
            token.as_str(),
        )
        .fetch_one(executor)
        .await
        // Never echo the token into the error id.
        .not_found_as("PasswordReset", "<token>")?;
        Ok(PasswordReset::from_persistence(
            PasswordResetId::new(rec.id),
            token.clone(),
            UserId::new(rec.user_id),
            rec.created_at,
            rec.expires_at,
            rec.used_at,
        ))
    }

    /// `false` when the link is used, expired or gone.
    pub async fn mark_used<'e, E>(executor: E, id: &PasswordResetId) -> DomainResult<bool>
    where
        E: PgExecutor<'e>,
    {
        let res = sqlx::query!(
            r#"
            UPDATE password_resets SET used_at = $2
            WHERE id = $1 AND used_at IS NULL AND expires_at > $2
            "#,
            id.as_str(),
            clock::now(),
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(res.rows_affected() > 0)
    }
}
