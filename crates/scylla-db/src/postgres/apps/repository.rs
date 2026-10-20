use crate::domain::agent::Agent;
use crate::domain::app::{App, AppCredential, AppKind, AppName};
use crate::domain::errors::DomainResult;
use crate::domain::ids::{AppId, OrganizationId};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scylla_auth::authz::{Grant, Principal};
use scylla_core::application::PrincipalAccess;
use scylla_core::application::app::AppRepository;
use sqlx::{PgExecutor, PgPool};
use tracing::instrument;

use super::super::error::{DbFieldExt, SqlxResultExt};
use super::super::{agents, app_secrets, grants};

#[derive(Clone)]
pub struct PgAppRepository {
    pool: PgPool,
}

impl PgAppRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AppRepository for PgAppRepository {
    #[instrument(skip_all, fields(app_id = %app.id(), org_id = %app.organization_id()))]
    async fn create_app(&self, app: &App, credential: &AppCredential) -> DomainResult<()> {
        let mut tx = self.pool.begin().await.to_domain()?;
        queries::create(&mut *tx, app).await?;
        app_secrets::insert(&mut *tx, credential).await?;
        tx.commit().await.to_domain()?;
        Ok(())
    }

    #[instrument(skip_all, fields(app_id = %app.id(), org_id = %app.organization_id()))]
    async fn provision_agent(
        &self,
        app: &App,
        credential: &AppCredential,
        agent: &Agent,
        grant: &Grant,
    ) -> DomainResult<()> {
        let mut tx = self.pool.begin().await.to_domain()?;
        queries::create(&mut *tx, app).await?;
        app_secrets::insert(&mut *tx, credential).await?;
        agents::insert(&mut *tx, agent).await?;
        grants::insert(&mut *tx, grant).await?;
        tx.commit().await.to_domain()?;
        Ok(())
    }

    #[instrument(skip_all, fields(app_id = %app.id(), org_id = %app.organization_id()))]
    async fn provision(&self, app: &App, grant: &Grant) -> DomainResult<()> {
        let mut tx = self.pool.begin().await.to_domain()?;
        queries::create(&mut *tx, app).await?;
        grants::insert(&mut *tx, grant).await?;
        tx.commit().await.to_domain()?;
        Ok(())
    }

    #[instrument(skip_all, fields(app_id = %id))]
    async fn find_by_id(&self, id: &AppId) -> DomainResult<App> {
        queries::find_by_id(&self.pool, id).await
    }

    #[instrument(skip_all, fields(org_id = %organization_id))]
    async fn find_trigger_runner(
        &self,
        organization_id: &OrganizationId,
    ) -> DomainResult<Option<AppId>> {
        queries::find_trigger_runner(&self.pool, organization_id).await
    }

    #[instrument(skip_all, fields(org_id = %organization_id))]
    async fn list_by_organization(
        &self,
        organization_id: &OrganizationId,
    ) -> DomainResult<Vec<App>> {
        queries::list_by_organization(&self.pool, organization_id).await
    }

    #[instrument(skip_all, fields(app_id = %id, active))]
    async fn set_active(&self, id: &AppId, active: bool) -> DomainResult<()> {
        queries::set_active(&self.pool, id, active).await
    }

    #[instrument(skip_all, fields(app_id = %id))]
    async fn delete(&self, id: &AppId) -> DomainResult<()> {
        queries::delete(&self.pool, id).await
    }

    #[instrument(skip_all, fields(app_id = %id))]
    async fn list_access(&self, id: &AppId) -> DomainResult<Vec<PrincipalAccess>> {
        grants::list_access(&self.pool, &Principal::App(id.clone())).await
    }
}

#[allow(clippy::wildcard_imports)]
pub mod queries {
    use super::*;

    fn row_into_app(
        id: String,
        organization_id: String,
        name: String,
        kind: String,
        is_active: bool,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> DomainResult<App> {
        let name = AppName::new(name).db_field("app name")?;
        let kind = AppKind::new(kind).db_field("app kind")?;
        Ok(App::from_persistence(
            AppId::new(id),
            OrganizationId::new(organization_id),
            name,
            kind,
            is_active,
            created_at,
            updated_at,
        ))
    }

    pub async fn create<'e, E>(executor: E, app: &App) -> DomainResult<()>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query!(
            r#"
            INSERT INTO apps (id, organization_id, name, kind, is_active, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
            app.id().as_str(),
            app.organization_id().as_str(),
            app.name().as_str(),
            app.kind().as_str(),
            app.is_active(),
            app.created_at(),
            app.updated_at(),
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(())
    }

    pub async fn find_by_id<'e, E>(executor: E, id: &AppId) -> DomainResult<App>
    where
        E: PgExecutor<'e>,
    {
        let rec = sqlx::query!(
            r#"
            SELECT id, organization_id, name, kind, is_active, created_at, updated_at
            FROM apps
            WHERE id = $1
            "#,
            id.as_str(),
        )
        .fetch_one(executor)
        .await
        .not_found_as("App", id)?;
        row_into_app(
            rec.id,
            rec.organization_id,
            rec.name,
            rec.kind,
            rec.is_active,
            rec.created_at,
            rec.updated_at,
        )
    }

    pub async fn list_by_organization<'e, E>(
        executor: E,
        organization_id: &OrganizationId,
    ) -> DomainResult<Vec<App>>
    where
        E: PgExecutor<'e>,
    {
        let rows = sqlx::query!(
            r#"
            SELECT id, organization_id, name, kind, is_active, created_at, updated_at
            FROM apps
            WHERE organization_id = $1
            ORDER BY created_at DESC
            "#,
            organization_id.as_str(),
        )
        .fetch_all(executor)
        .await
        .to_domain()?;
        rows.into_iter()
            .map(|r| {
                row_into_app(
                    r.id,
                    r.organization_id,
                    r.name,
                    r.kind,
                    r.is_active,
                    r.created_at,
                    r.updated_at,
                )
            })
            .collect()
    }

    pub async fn find_trigger_runner<'e, E>(
        executor: E,
        organization_id: &OrganizationId,
    ) -> DomainResult<Option<AppId>>
    where
        E: PgExecutor<'e>,
    {
        let id = sqlx::query_scalar!(
            "SELECT id FROM apps WHERE organization_id = $1 AND kind = 'trigger_runner'",
            organization_id.as_str(),
        )
        .fetch_optional(executor)
        .await
        .to_domain()?;
        Ok(id.map(AppId::new))
    }

    pub async fn set_active<'e, E>(executor: E, id: &AppId, active: bool) -> DomainResult<()>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query!(
            "UPDATE apps SET is_active = $2, updated_at = NOW() WHERE id = $1",
            id.as_str(),
            active,
        )
        .execute(executor)
        .await
        .to_domain()?;
        Ok(())
    }

    pub async fn delete<'e, E>(executor: E, id: &AppId) -> DomainResult<()>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query!("DELETE FROM apps WHERE id = $1", id.as_str())
            .execute(executor)
            .await
            .to_domain()?;
        Ok(())
    }
}
