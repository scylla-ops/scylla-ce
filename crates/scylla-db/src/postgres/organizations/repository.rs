use crate::domain::errors::DomainResult;
use crate::domain::ids::{OrganizationId, UserId};
use crate::domain::organization::Organization;
use crate::domain::organization::{OrganizationDescription, OrganizationName};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scylla_auth::authz::Grant;
use scylla_core::application::OrganizationRepository;
use scylla_core::application::pagination::{PaginatedResult, PaginationParams};
use sqlx::{PgExecutor, PgPool};
use tracing::instrument;

use super::super::error::{DbFieldExt, SqlxResultExt};
use super::super::grants;
use super::super::version::{from_db, to_db, written};

#[derive(Clone)]
pub struct PgOrganizationRepository {
    pool: PgPool,
}

impl PgOrganizationRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl OrganizationRepository for PgOrganizationRepository {
    #[instrument(skip_all, fields(org_id = %organization.id()))]
    async fn create(&self, organization: &Organization) -> DomainResult<Organization> {
        queries::create(&self.pool, organization).await
    }

    #[instrument(skip_all, fields(org_id = %organization.id()))]
    async fn provision_with_owner(
        &self,
        organization: &Organization,
        grant: &Grant,
    ) -> DomainResult<()> {
        let mut tx = self.pool.begin().await.to_domain()?;
        queries::create(&mut *tx, organization).await?;
        grants::insert(&mut *tx, grant).await?;
        tx.commit().await.to_domain()?;
        Ok(())
    }

    #[instrument(skip_all, fields(org_id = %org_id))]
    async fn list_principals(
        &self,
        org_id: &OrganizationId,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<UserId>> {
        let params = pagination.copied().unwrap_or_default();
        let total = queries::count_principals(&self.pool, org_id).await?;
        let items = queries::list_principals_page(&self.pool, org_id, &params).await?;
        Ok(PaginatedResult::new(items, &params, total))
    }

    #[instrument(skip_all, fields(org_id = %org_id))]
    async fn count_members(&self, org_id: &OrganizationId) -> DomainResult<u64> {
        queries::count_principals(&self.pool, org_id).await
    }

    #[instrument(skip_all, fields(org_id = %org_id, user_id = %user_id))]
    async fn has_member(&self, org_id: &OrganizationId, user_id: &UserId) -> DomainResult<bool> {
        queries::has_principal(&self.pool, org_id, user_id).await
    }

    #[instrument(skip_all, fields(user_id = %user_id))]
    async fn list_for_user(
        &self,
        user_id: &UserId,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Organization>> {
        let params = pagination.copied().unwrap_or_default();
        let total = queries::count_for_user(&self.pool, user_id).await?;
        let items = queries::list_for_user_page(&self.pool, user_id, &params).await?;
        Ok(PaginatedResult::new(items, &params, total))
    }

    #[instrument(skip_all, fields(org_id = %id))]
    async fn find_by_id(&self, id: &OrganizationId) -> DomainResult<Organization> {
        queries::find_by_id(&self.pool, id).await
    }

    #[instrument(skip_all, fields(org_id = %organization.id(), version = organization.version()))]
    async fn update(&self, organization: &Organization) -> DomainResult<Organization> {
        let updated = queries::update(&self.pool, organization).await?;
        written(
            updated,
            "Organization",
            organization.id(),
            queries::find_by_id(&self.pool, organization.id()),
        )
        .await
    }

    #[instrument(skip_all, fields(org_id = %organization.id(), version = organization.version()))]
    async fn delete(&self, organization: &Organization) -> DomainResult<()> {
        let deleted = queries::delete(&self.pool, organization)
            .await?
            .then_some(());
        written(
            deleted,
            "Organization",
            organization.id(),
            queries::find_by_id(&self.pool, organization.id()),
        )
        .await
    }

    #[instrument(skip(self, pagination))]
    async fn list_all(
        &self,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Organization>> {
        let params = pagination.copied().unwrap_or_default();
        let total = queries::count_all(&self.pool).await?;
        let items = queries::list_page(&self.pool, &params, false).await?;
        Ok(PaginatedResult::new(items, &params, total))
    }
}

#[allow(clippy::wildcard_imports)]
pub mod queries {
    use super::*;

    pub async fn count_principals<'e, E>(executor: E, org_id: &OrganizationId) -> DomainResult<u64>
    where
        E: PgExecutor<'e>,
    {
        let row = sqlx::query!(
            r#"
            SELECT COUNT(DISTINCT g.principal_id) AS "count!"
            FROM grants g
            WHERE g.principal_kind = 'user'
              AND ((g.scope_kind = 'organization' AND g.scope_id = $1)
                OR (g.scope_kind = 'project' AND g.scope_id IN
                      (SELECT id FROM projects WHERE organization_id = $1)))
            "#,
            org_id.as_str(),
        )
        .fetch_one(executor)
        .await
        .to_domain()?;
        Ok(u64::try_from(row.count).unwrap_or(0))
    }

    pub async fn has_principal<'e, E>(
        executor: E,
        org_id: &OrganizationId,
        user_id: &UserId,
    ) -> DomainResult<bool>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query_scalar!(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM grants g
                WHERE g.principal_kind = 'user' AND g.principal_id = $2
                  AND ((g.scope_kind = 'organization' AND g.scope_id = $1)
                    OR (g.scope_kind = 'project' AND g.scope_id IN
                          (SELECT id FROM projects WHERE organization_id = $1)))
            ) AS "member!"
            "#,
            org_id.as_str(),
            user_id.as_str(),
        )
        .fetch_one(executor)
        .await
        .to_domain()
    }

    pub async fn list_principals_page<'e, E>(
        executor: E,
        org_id: &OrganizationId,
        params: &PaginationParams,
    ) -> DomainResult<Vec<UserId>>
    where
        E: PgExecutor<'e>,
    {
        let limit = i64::try_from(params.limit()).unwrap_or(i64::MAX);
        let offset = i64::try_from(params.offset()).unwrap_or(i64::MAX);
        let rows = sqlx::query!(
            r#"
            SELECT g.principal_id, MAX(g.created_at) AS "granted_at!"
            FROM grants g
            WHERE g.principal_kind = 'user'
              AND ((g.scope_kind = 'organization' AND g.scope_id = $1)
                OR (g.scope_kind = 'project' AND g.scope_id IN
                      (SELECT id FROM projects WHERE organization_id = $1)))
            GROUP BY g.principal_id
            ORDER BY "granted_at!" DESC
            LIMIT $2 OFFSET $3
            "#,
            org_id.as_str(),
            limit,
            offset,
        )
        .fetch_all(executor)
        .await
        .to_domain()?;
        Ok(rows
            .into_iter()
            .map(|r| UserId::new(r.principal_id))
            .collect())
    }

    pub async fn count_for_user<'e, E>(executor: E, user_id: &UserId) -> DomainResult<u64>
    where
        E: PgExecutor<'e>,
    {
        let row = sqlx::query!(
            r#"
            SELECT COUNT(*) AS "count!" FROM organizations o
            WHERE o.id IN (
                SELECT g.scope_id FROM grants g
                WHERE g.principal_kind = 'user' AND g.principal_id = $1
                  AND g.scope_kind = 'organization'
              )
              OR o.id IN (
                SELECT p.organization_id FROM projects p
                JOIN grants g ON g.scope_kind = 'project' AND g.scope_id = p.id
                WHERE g.principal_kind = 'user' AND g.principal_id = $1
              )
            "#,
            user_id.as_str(),
        )
        .fetch_one(executor)
        .await
        .to_domain()?;
        Ok(u64::try_from(row.count).unwrap_or(0))
    }

    pub async fn list_for_user_page<'e, E>(
        executor: E,
        user_id: &UserId,
        params: &PaginationParams,
    ) -> DomainResult<Vec<Organization>>
    where
        E: PgExecutor<'e>,
    {
        let limit = i64::try_from(params.limit()).unwrap_or(i64::MAX);
        let offset = i64::try_from(params.offset()).unwrap_or(i64::MAX);
        let rows = sqlx::query!(
            r#"
            SELECT id, name, description, is_active, created_at, updated_at, version
            FROM organizations o
            WHERE o.id IN (
                SELECT g.scope_id FROM grants g
                WHERE g.principal_kind = 'user' AND g.principal_id = $1
                  AND g.scope_kind = 'organization'
              )
              OR o.id IN (
                SELECT p.organization_id FROM projects p
                JOIN grants g ON g.scope_kind = 'project' AND g.scope_id = p.id
                WHERE g.principal_kind = 'user' AND g.principal_id = $1
              )
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
            "#,
            user_id.as_str(),
            limit,
            offset,
        )
        .fetch_all(executor)
        .await
        .to_domain()?;
        rows.into_iter()
            .map(|r| {
                row_into_org(
                    r.id,
                    r.name,
                    r.description,
                    r.is_active,
                    r.created_at,
                    r.updated_at,
                    r.version,
                )
            })
            .collect()
    }

    fn row_into_org(
        id: String,
        name: String,
        description: Option<String>,
        is_active: bool,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        version: i64,
    ) -> DomainResult<Organization> {
        let name = OrganizationName::new(name).db_field("org name")?;
        let description = description
            .map(OrganizationDescription::new)
            .transpose()
            .db_field("org description")?;
        Ok(Organization::from_persistence(
            OrganizationId::new(id),
            name,
            description,
            is_active,
            created_at,
            updated_at,
            from_db(version),
        ))
    }

    pub async fn create<'e, E>(executor: E, org: &Organization) -> DomainResult<Organization>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query!(
            r#"
            INSERT INTO organizations (id, name, description, is_active, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
            org.id().as_str(),
            org.name().as_str(),
            org.description().map(OrganizationDescription::as_str),
            org.is_active(),
            org.created_at(),
            org.updated_at(),
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(org.clone())
    }

    pub async fn find_by_id<'e, E>(executor: E, id: &OrganizationId) -> DomainResult<Organization>
    where
        E: PgExecutor<'e>,
    {
        let rec = sqlx::query!(
            r#"
            SELECT id, name, description, is_active, created_at, updated_at, version
            FROM organizations
            WHERE id = $1
            "#,
            id.as_str(),
        )
        .fetch_one(executor)
        .await
        .not_found_as("Organization", id)?;
        row_into_org(
            rec.id,
            rec.name,
            rec.description,
            rec.is_active,
            rec.created_at,
            rec.updated_at,
            rec.version,
        )
    }

    /// `None` when no row carries the staged version.
    pub async fn update<'e, E>(
        executor: E,
        org: &Organization,
    ) -> DomainResult<Option<Organization>>
    where
        E: PgExecutor<'e>,
    {
        let rec = sqlx::query!(
            r#"
            UPDATE organizations
            SET name = $2,
                description = $3,
                is_active = $4,
                updated_at = $5,
                version = version + 1
            WHERE id = $1 AND version = $6
            RETURNING id, name, description, is_active, created_at, updated_at, version
            "#,
            org.id().as_str(),
            org.name().as_str(),
            org.description().map(OrganizationDescription::as_str),
            org.is_active(),
            org.updated_at(),
            to_db(org.version()),
        )
        .fetch_optional(executor)
        .await
        .to_domain()?;
        rec.map(|r| {
            row_into_org(
                r.id,
                r.name,
                r.description,
                r.is_active,
                r.created_at,
                r.updated_at,
                r.version,
            )
        })
        .transpose()
    }

    /// `false` when no row carries the staged version.
    pub async fn delete<'e, E>(executor: E, org: &Organization) -> DomainResult<bool>
    where
        E: PgExecutor<'e>,
    {
        let res = sqlx::query!(
            "DELETE FROM organizations WHERE id = $1 AND version = $2",
            org.id().as_str(),
            to_db(org.version()),
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
        let row = sqlx::query!(r#"SELECT COUNT(*) AS "count!" FROM organizations"#)
            .fetch_one(executor)
            .await
            .to_domain()?;
        Ok(u64::try_from(row.count).unwrap_or(0))
    }

    pub async fn list_page<'e, E>(
        executor: E,
        params: &PaginationParams,
        only_active: bool,
    ) -> DomainResult<Vec<Organization>>
    where
        E: PgExecutor<'e>,
    {
        let limit = i64::try_from(params.limit()).unwrap_or(i64::MAX);
        let offset = i64::try_from(params.offset()).unwrap_or(i64::MAX);
        let rows = sqlx::query!(
            r#"
            SELECT id, name, description, is_active, created_at, updated_at, version
            FROM organizations
            WHERE NOT $3 OR is_active
            ORDER BY created_at DESC
            LIMIT $1 OFFSET $2
            "#,
            limit,
            offset,
            only_active,
        )
        .fetch_all(executor)
        .await
        .to_domain()?;
        rows.into_iter()
            .map(|r| {
                row_into_org(
                    r.id,
                    r.name,
                    r.description,
                    r.is_active,
                    r.created_at,
                    r.updated_at,
                    r.version,
                )
            })
            .collect()
    }
}
