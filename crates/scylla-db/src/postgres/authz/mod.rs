use crate::domain::errors::DomainResult;
use crate::domain::ids::{AppId, OrganizationId, PipelineId, ProjectId};
use crate::domain::permission::ResourceRef;
use async_trait::async_trait;
use scylla_auth::authz::{AuthzEntityProvider, ResourceAncestors};
use sqlx::PgPool;
use tracing::instrument;

use super::error::SqlxResultExt;

#[derive(Clone)]
pub struct PgAuthzEntityProvider {
    pool: PgPool,
}

impl PgAuthzEntityProvider {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AuthzEntityProvider for PgAuthzEntityProvider {
    #[instrument(skip_all, fields(resource = %resource))]
    async fn resource_ancestors(&self, resource: &ResourceRef) -> DomainResult<ResourceAncestors> {
        match resource {
            ResourceRef::Project(id) => {
                let row = sqlx::query!(
                    "SELECT organization_id FROM projects WHERE id = $1",
                    id.as_str(),
                )
                .fetch_optional(&self.pool)
                .await
                .to_domain()?;
                Ok(ResourceAncestors {
                    organization: row.map(|r| OrganizationId::new(r.organization_id)),
                    ..Default::default()
                })
            }
            ResourceRef::Pipeline(id) => {
                // `!`: sqlx cannot prove the inner join yields a row; both FKs are NOT NULL.
                let row = sqlx::query!(
                    "SELECT pl.project_id AS \"project_id!\", pr.organization_id AS \"organization_id!\" \
                     FROM pipelines pl JOIN projects pr ON pr.id = pl.project_id \
                     WHERE pl.id = $1",
                    id.as_str(),
                )
                .fetch_optional(&self.pool)
                .await
                .to_domain()?;
                match row {
                    Some(r) => Ok(ResourceAncestors {
                        organization: Some(OrganizationId::new(r.organization_id)),
                        project: Some(ProjectId::new(r.project_id)),
                        pipeline: None,
                        app: None,
                    }),
                    None => Ok(ResourceAncestors::default()),
                }
            }
            ResourceRef::Job(id) => {
                let row = sqlx::query!(
                    "SELECT j.pipeline_id AS \"pipeline_id!\", pl.project_id AS \"project_id!\", \
                            pr.organization_id AS \"organization_id!\" \
                     FROM jobs j \
                     JOIN pipelines pl ON pl.id = j.pipeline_id \
                     JOIN projects pr ON pr.id = pl.project_id \
                     WHERE j.id = $1",
                    id.as_str(),
                )
                .fetch_optional(&self.pool)
                .await
                .to_domain()?;
                match row {
                    Some(r) => Ok(ResourceAncestors {
                        organization: Some(OrganizationId::new(r.organization_id)),
                        project: Some(ProjectId::new(r.project_id)),
                        pipeline: Some(PipelineId::new(r.pipeline_id)),
                        app: None,
                    }),
                    None => Ok(ResourceAncestors::default()),
                }
            }
            ResourceRef::Secret(id) => {
                let row = sqlx::query!(
                    "SELECT s.project_id AS \"project_id!\", pr.organization_id AS \"organization_id!\" \
                     FROM project_secrets s JOIN projects pr ON pr.id = s.project_id \
                     WHERE s.id = $1",
                    id.as_str(),
                )
                .fetch_optional(&self.pool)
                .await
                .to_domain()?;
                match row {
                    Some(r) => Ok(ResourceAncestors {
                        organization: Some(OrganizationId::new(r.organization_id)),
                        project: Some(ProjectId::new(r.project_id)),
                        pipeline: None,
                        app: None,
                    }),
                    None => Ok(ResourceAncestors::default()),
                }
            }
            ResourceRef::Trigger(id) => {
                let row = sqlx::query!(
                    "SELECT t.pipeline_id AS \"pipeline_id!\", pl.project_id AS \"project_id!\", \
                            pr.organization_id AS \"organization_id!\" \
                     FROM pipeline_triggers t \
                     JOIN pipelines pl ON pl.id = t.pipeline_id \
                     JOIN projects pr ON pr.id = pl.project_id \
                     WHERE t.id = $1",
                    id.as_str(),
                )
                .fetch_optional(&self.pool)
                .await
                .to_domain()?;
                match row {
                    Some(r) => Ok(ResourceAncestors {
                        organization: Some(OrganizationId::new(r.organization_id)),
                        project: Some(ProjectId::new(r.project_id)),
                        pipeline: Some(PipelineId::new(r.pipeline_id)),
                        app: None,
                    }),
                    None => Ok(ResourceAncestors::default()),
                }
            }
            ResourceRef::App(id) => {
                let row = sqlx::query!(
                    "SELECT organization_id FROM apps WHERE id = $1",
                    id.as_str(),
                )
                .fetch_optional(&self.pool)
                .await
                .to_domain()?;
                Ok(ResourceAncestors {
                    organization: row.map(|r| OrganizationId::new(r.organization_id)),
                    ..Default::default()
                })
            }
            ResourceRef::AppSecret(id) => {
                let row = sqlx::query!(
                    "SELECT s.app_id AS \"app_id!\", a.organization_id AS \"organization_id!\" \
                     FROM app_secrets s JOIN apps a ON a.id = s.app_id \
                     WHERE s.id = $1",
                    id.as_str(),
                )
                .fetch_optional(&self.pool)
                .await
                .to_domain()?;
                match row {
                    Some(r) => Ok(ResourceAncestors {
                        organization: Some(OrganizationId::new(r.organization_id)),
                        app: Some(AppId::new(r.app_id)),
                        ..Default::default()
                    }),
                    None => Ok(ResourceAncestors::default()),
                }
            }
            ResourceRef::Grant(id) => {
                let row = sqlx::query!(
                    "SELECT g.scope_kind, g.scope_id, pr.organization_id AS \"project_organization_id?\" \
                     FROM grants g \
                     LEFT JOIN projects pr ON g.scope_kind = 'project' AND pr.id = g.scope_id \
                     WHERE g.id = $1",
                    id.as_str(),
                )
                .fetch_optional(&self.pool)
                .await
                .to_domain()?;
                Ok(match row {
                    Some(r) if r.scope_kind == "project" => ResourceAncestors {
                        organization: r.project_organization_id.map(OrganizationId::new),
                        project: Some(ProjectId::new(r.scope_id)),
                        ..Default::default()
                    },
                    Some(r) if r.scope_kind == "organization" => ResourceAncestors {
                        organization: Some(OrganizationId::new(r.scope_id)),
                        ..Default::default()
                    },
                    _ => ResourceAncestors::default(),
                })
            }
            ResourceRef::Role(id) => {
                let owner = sqlx::query_scalar!(
                    "SELECT owner_org_id FROM roles WHERE id = $1",
                    id.as_str(),
                )
                .fetch_optional(&self.pool)
                .await
                .to_domain()?
                .flatten();
                Ok(ResourceAncestors {
                    organization: owner.map(OrganizationId::new),
                    ..Default::default()
                })
            }
            _ => Ok(ResourceAncestors::default()),
        }
    }

    #[instrument(skip_all, fields(app_id = %app))]
    async fn app_is_active(&self, app: &AppId) -> DomainResult<bool> {
        let row = sqlx::query!("SELECT is_active FROM apps WHERE id = $1", app.as_str())
            .fetch_optional(&self.pool)
            .await
            .to_domain()?;
        // No row: the App was deleted; inactive, so an in-flight stream is denied.
        Ok(row.is_some_and(|r| r.is_active))
    }

    async fn policy_version(&self) -> DomainResult<i64> {
        sqlx::query_scalar!("SELECT version FROM authz_version")
            .fetch_one(&self.pool)
            .await
            .to_domain()
    }
}

#[cfg(test)]
mod tests;
