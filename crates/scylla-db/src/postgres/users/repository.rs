use crate::domain::errors::DomainResult;
use crate::domain::ids::UserId;
use crate::domain::user::User;
use crate::domain::user::{DisplayName, Email, PasswordHash, Username};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scylla_core::application::UserRepository;
use scylla_core::application::pagination::{PaginatedResult, PaginationParams};
use sqlx::{PgExecutor, PgPool};
use tracing::instrument;

use super::super::error::{DbFieldExt, SqlxResultExt};
use super::super::version::{from_db, to_db, written};

#[derive(Clone)]
pub struct PgUserRepository {
    pool: PgPool,
}

impl PgUserRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserRepository for PgUserRepository {
    #[instrument(skip_all, fields(user_id = %user.id()))]
    async fn create(&self, user: &User) -> DomainResult<User> {
        queries::create(&self.pool, user).await
    }

    #[instrument(skip_all, fields(user_id = %id))]
    async fn find_by_id(&self, id: &UserId) -> DomainResult<User> {
        queries::find_by_id(&self.pool, id).await
    }

    #[instrument(skip_all, fields(n = ids.len()))]
    async fn find_by_ids(&self, ids: &[UserId]) -> DomainResult<Vec<User>> {
        queries::find_by_ids(&self.pool, ids).await
    }

    #[instrument(skip_all)]
    async fn find_by_username(&self, username: &Username) -> DomainResult<User> {
        queries::find_by_username(&self.pool, username).await
    }

    #[instrument(skip_all)]
    async fn find_by_email(&self, email: &Email) -> DomainResult<User> {
        queries::find_by_email(&self.pool, email).await
    }

    #[instrument(skip_all, fields(user_id = %user.id(), version = user.version()))]
    async fn update(&self, user: &User) -> DomainResult<User> {
        let updated = queries::update(&self.pool, user).await?;
        written(
            updated,
            "User",
            user.id(),
            queries::find_by_id(&self.pool, user.id()),
        )
        .await
    }

    #[instrument(skip_all, fields(user_id = %user.id(), version = user.version()))]
    async fn delete(&self, user: &User) -> DomainResult<()> {
        let deleted = queries::delete(&self.pool, user).await?.then_some(());
        written(
            deleted,
            "User",
            user.id(),
            queries::find_by_id(&self.pool, user.id()),
        )
        .await
    }

    #[instrument(skip(self, pagination))]
    async fn list_all(
        &self,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<User>> {
        let params = pagination.copied().unwrap_or_default();
        let total = queries::count_all(&self.pool).await?;
        let items = queries::list_page(&self.pool, &params).await?;
        Ok(PaginatedResult::new(items, &params, total))
    }
}

#[allow(clippy::wildcard_imports)]
pub mod queries {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn row_into_user(
        id: String,
        username: String,
        email: Option<String>,
        display_name: Option<String>,
        password_hash: String,
        is_active: bool,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        version: i64,
    ) -> DomainResult<User> {
        let username = Username::new(username).db_field("username")?;
        let email = email.map(Email::new).transpose().db_field("email")?;
        let display_name = display_name
            .map(DisplayName::new)
            .transpose()
            .db_field("display name")?;
        let password_hash = PasswordHash::new(password_hash).db_field("password hash")?;
        Ok(User::from_persistence(
            UserId::new(id),
            username,
            email,
            display_name,
            password_hash,
            is_active,
            created_at,
            updated_at,
            from_db(version),
        ))
    }

    pub async fn create<'e, E>(executor: E, user: &User) -> DomainResult<User>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query!(
            r#"
            INSERT INTO users (id, username, email, display_name, password_hash, is_active, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
            user.id().as_str(),
            user.username().as_str(),
            user.email().map(Email::as_str),
            user.display_name().map(DisplayName::as_str),
            user.password_hash().as_str(),
            user.is_active(),
            user.created_at(),
            user.updated_at(),
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(user.clone())
    }

    pub async fn find_by_id<'e, E>(executor: E, id: &UserId) -> DomainResult<User>
    where
        E: PgExecutor<'e>,
    {
        let rec = sqlx::query!(
            r#"
            SELECT id, username, email, display_name, password_hash, is_active, created_at, updated_at, version
            FROM users
            WHERE id = $1
            "#,
            id.as_str(),
        )
        .fetch_one(executor)
        .await
        .not_found_as("User", id)?;
        row_into_user(
            rec.id,
            rec.username,
            rec.email,
            rec.display_name,
            rec.password_hash,
            rec.is_active,
            rec.created_at,
            rec.updated_at,
            rec.version,
        )
    }

    pub async fn find_by_ids<'e, E>(executor: E, ids: &[UserId]) -> DomainResult<Vec<User>>
    where
        E: PgExecutor<'e>,
    {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let id_strs: Vec<String> = ids.iter().map(|i| i.as_str().to_owned()).collect();
        let rows = sqlx::query!(
            r#"
            SELECT id, username, email, display_name, password_hash, is_active, created_at, updated_at, version
            FROM users
            WHERE id = ANY($1::text[])
            "#,
            &id_strs,
        )
        .fetch_all(executor)
        .await
        .to_domain()?;
        rows.into_iter()
            .map(|r| {
                row_into_user(
                    r.id,
                    r.username,
                    r.email,
                    r.display_name,
                    r.password_hash,
                    r.is_active,
                    r.created_at,
                    r.updated_at,
                    r.version,
                )
            })
            .collect()
    }

    pub async fn find_by_username<'e, E>(executor: E, username: &Username) -> DomainResult<User>
    where
        E: PgExecutor<'e>,
    {
        let rec = sqlx::query!(
            r#"
            SELECT id, username, email, display_name, password_hash, is_active, created_at, updated_at, version
            FROM users
            WHERE username = $1
            "#,
            username.as_str(),
        )
        .fetch_one(executor)
        .await
        // Never echo the username into the error id (account-existence oracle).
        .not_found_as("User", "<username>")?;
        row_into_user(
            rec.id,
            rec.username,
            rec.email,
            rec.display_name,
            rec.password_hash,
            rec.is_active,
            rec.created_at,
            rec.updated_at,
            rec.version,
        )
    }

    pub async fn find_by_email<'e, E>(executor: E, email: &Email) -> DomainResult<User>
    where
        E: PgExecutor<'e>,
    {
        let rec = sqlx::query!(
            r#"
            SELECT id, username, email, display_name, password_hash, is_active, created_at, updated_at, version
            FROM users
            WHERE email = $1
            "#,
            email.as_str(),
        )
        .fetch_one(executor)
        .await
        // Never echo the email into the error id (account-existence oracle).
        .not_found_as("User", "<email>")?;
        row_into_user(
            rec.id,
            rec.username,
            rec.email,
            rec.display_name,
            rec.password_hash,
            rec.is_active,
            rec.created_at,
            rec.updated_at,
            rec.version,
        )
    }

    /// `None` when no row carries the staged version.
    pub async fn update<'e, E>(executor: E, user: &User) -> DomainResult<Option<User>>
    where
        E: PgExecutor<'e>,
    {
        let rec = sqlx::query!(
            r#"
            UPDATE users
            SET username = $2,
                email = $3,
                display_name = $4,
                password_hash = $5,
                is_active = $6,
                updated_at = $7,
                version = version + 1
            WHERE id = $1 AND version = $8
            RETURNING id, username, email, display_name, password_hash, is_active, created_at, updated_at, version
            "#,
            user.id().as_str(),
            user.username().as_str(),
            user.email().map(Email::as_str),
            user.display_name().map(DisplayName::as_str),
            user.password_hash().as_str(),
            user.is_active(),
            user.updated_at(),
            to_db(user.version()),
        )
        .fetch_optional(executor)
        .await
        .to_domain()?;
        rec.map(|r| {
            row_into_user(
                r.id,
                r.username,
                r.email,
                r.display_name,
                r.password_hash,
                r.is_active,
                r.created_at,
                r.updated_at,
                r.version,
            )
        })
        .transpose()
    }

    /// `false` when no row carries the staged version.
    pub async fn delete<'e, E>(executor: E, user: &User) -> DomainResult<bool>
    where
        E: PgExecutor<'e>,
    {
        let res = sqlx::query!(
            "DELETE FROM users WHERE id = $1 AND version = $2",
            user.id().as_str(),
            to_db(user.version()),
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn count_all<'e, E>(executor: E) -> DomainResult<u64>
    where
        E: PgExecutor<'e>,
    {
        let row = sqlx::query!(r#"SELECT COUNT(*) AS "count!" FROM users"#)
            .fetch_one(executor)
            .await
            .to_domain()?;
        Ok(u64::try_from(row.count).unwrap_or(0))
    }

    pub async fn list_page<'e, E>(executor: E, params: &PaginationParams) -> DomainResult<Vec<User>>
    where
        E: PgExecutor<'e>,
    {
        let limit = i64::try_from(params.limit()).unwrap_or(i64::MAX);
        let offset = i64::try_from(params.offset()).unwrap_or(i64::MAX);
        let rows = sqlx::query!(
            r#"
            SELECT id, username, email, display_name, password_hash, is_active, created_at, updated_at, version
            FROM users
            ORDER BY created_at DESC
            LIMIT $1 OFFSET $2
            "#,
            limit,
            offset,
        )
        .fetch_all(executor)
        .await
        .to_domain()?;
        rows.into_iter()
            .map(|r| {
                row_into_user(
                    r.id,
                    r.username,
                    r.email,
                    r.display_name,
                    r.password_hash,
                    r.is_active,
                    r.created_at,
                    r.updated_at,
                    r.version,
                )
            })
            .collect()
    }
}
