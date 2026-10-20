use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{AppId, OrganizationId, ProjectId, UserId};
use crate::domain::organization::OrganizationName;
use crate::domain::project::ProjectName;
use crate::domain::role::{RoleDisplayName, RoleName};
use async_trait::async_trait;
use scylla_auth::authz::{Grant, GrantRepository, Principal, Scope};
use scylla_core::application::PrincipalAccess;
use sqlx::{PgExecutor, PgPool};
use tracing::instrument;

use super::error::{DbFieldExt, SqlxResultExt};

const SCOPE_SYSTEM: &str = "system";
const SCOPE_ORGANIZATION: &str = "organization";
const SCOPE_PROJECT: &str = "project";
/// The column is NOT NULL and there is one System root, so the id is constant and ignored on read.
const SYSTEM_SCOPE_ID: &str = "system";
const PRINCIPAL_USER: &str = "user";
const PRINCIPAL_APP: &str = "app";

fn scope_from_row(kind: &str, id: String) -> DomainResult<Scope> {
    match kind {
        SCOPE_SYSTEM => Ok(Scope::System),
        SCOPE_ORGANIZATION => Ok(Scope::Organization(OrganizationId::new(id))),
        SCOPE_PROJECT => Ok(Scope::Project(ProjectId::new(id))),
        other => Err(DomainError::Infrastructure(format!(
            "unknown grant scope_kind '{other}'"
        ))),
    }
}

/// Returns the stored id: the existing one when the same grant is already there.
pub async fn insert<'e, E>(executor: E, grant: &Grant) -> DomainResult<String>
where
    E: PgExecutor<'e>,
{
    let (scope_kind, scope_id) = match &grant.scope {
        Scope::System => (SCOPE_SYSTEM, SYSTEM_SCOPE_ID),
        Scope::Organization(id) => (SCOPE_ORGANIZATION, id.as_str()),
        Scope::Project(id) => (SCOPE_PROJECT, id.as_str()),
    };
    sqlx::query_scalar!(
        "INSERT INTO grants (id, principal_kind, principal_id, role_id, scope_kind, scope_id) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (principal_kind, principal_id, role_id, scope_kind, scope_id) \
         DO UPDATE SET id = grants.id \
         RETURNING id",
        grant.id.as_str(),
        grant.principal.kind().as_str(),
        grant.principal.id(),
        grant.role.as_str(),
        scope_kind,
        scope_id,
    )
    .fetch_one(executor)
    .await
    .to_domain()
}

/// System-scoped grants stay out of reach: an org admin must not strip a platform operator.
pub async fn delete_under_scope<'e, E>(
    executor: E,
    principal: &Principal,
    scope: &Scope,
) -> DomainResult<u64>
where
    E: PgExecutor<'e>,
{
    let result = match scope {
        Scope::System => {
            sqlx::query!(
                "DELETE FROM grants \
                 WHERE principal_kind = $1 AND principal_id = $2 AND scope_kind <> $3",
                principal.kind().as_str(),
                principal.id(),
                SCOPE_SYSTEM,
            )
            .execute(executor)
            .await
        }
        Scope::Organization(org_id) => {
            sqlx::query!(
                "DELETE FROM grants \
                 WHERE principal_kind = $1 AND principal_id = $2 \
                   AND ((scope_kind = $3 AND scope_id = $4) \
                     OR (scope_kind = $5 AND scope_id IN \
                           (SELECT id FROM projects WHERE organization_id = $4)))",
                principal.kind().as_str(),
                principal.id(),
                SCOPE_ORGANIZATION,
                org_id.as_str(),
                SCOPE_PROJECT,
            )
            .execute(executor)
            .await
        }
        Scope::Project(project_id) => {
            sqlx::query!(
                "DELETE FROM grants \
                 WHERE principal_kind = $1 AND principal_id = $2 \
                   AND scope_kind = $3 AND scope_id = $4",
                principal.kind().as_str(),
                principal.id(),
                SCOPE_PROJECT,
                project_id.as_str(),
            )
            .execute(executor)
            .await
        }
    };
    Ok(result.to_domain()?.rows_affected())
}

pub async fn list_access<'e, E>(
    executor: E,
    principal: &Principal,
) -> DomainResult<Vec<PrincipalAccess>>
where
    E: PgExecutor<'e>,
{
    let rows = sqlx::query!(
        r#"
        SELECT g.id, g.scope_kind, g.scope_id, g.role_id, r.name AS role_name,
               o.id AS "organization_id?", o.name AS "organization_name?",
               p.name AS "project_name?"
        FROM grants g
        JOIN roles r ON r.id = g.role_id
        LEFT JOIN projects p ON g.scope_kind = 'project' AND p.id = g.scope_id
        LEFT JOIN organizations o ON o.id = CASE g.scope_kind
            WHEN 'organization' THEN g.scope_id
            WHEN 'project' THEN p.organization_id
        END
        WHERE g.principal_kind = $1 AND g.principal_id = $2
        ORDER BY g.scope_kind <> 'system', o.name, o.id,
                 p.name NULLS FIRST, p.id NULLS FIRST, r.name, g.id
        "#,
        principal.kind().as_str(),
        principal.id(),
    )
    .fetch_all(executor)
    .await
    .to_domain()?;
    rows.into_iter()
        .map(|r| {
            Ok(PrincipalAccess {
                grant_id: r.id,
                scope: scope_from_row(&r.scope_kind, r.scope_id)?,
                organization_id: r.organization_id.map(OrganizationId::new),
                organization_name: r
                    .organization_name
                    .map(OrganizationName::new)
                    .transpose()
                    .db_field("organization name")?,
                project_name: r
                    .project_name
                    .map(ProjectName::new)
                    .transpose()
                    .db_field("project name")?,
                role: RoleName::new(r.role_id).db_field("role id")?,
                role_name: RoleDisplayName::new(r.role_name).db_field("role name")?,
            })
        })
        .collect()
}

#[derive(Clone)]
pub struct PgGrantRepository {
    pool: PgPool,
}

impl PgGrantRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl GrantRepository for PgGrantRepository {
    #[instrument(skip(self))]
    async fn list_all(&self) -> DomainResult<Vec<Grant>> {
        let rows = sqlx::query!(
            "SELECT id, principal_kind, principal_id, role_id, scope_kind, scope_id FROM grants",
        )
        .fetch_all(&self.pool)
        .await
        .to_domain()?;

        rows.into_iter()
            .map(|r| {
                let principal = match r.principal_kind.as_str() {
                    PRINCIPAL_USER => Principal::User(UserId::new(r.principal_id)),
                    PRINCIPAL_APP => Principal::App(AppId::new(r.principal_id)),
                    other => {
                        return Err(DomainError::Infrastructure(format!(
                            "unknown grant principal_kind '{other}'"
                        )));
                    }
                };
                Ok(Grant {
                    id: r.id,
                    principal,
                    role: RoleName::new(r.role_id)?,
                    scope: scope_from_row(&r.scope_kind, r.scope_id)?,
                })
            })
            .collect()
    }

    #[instrument(skip_all, fields(grant_id = %grant.id))]
    async fn create(&self, grant: &Grant) -> DomainResult<Grant> {
        let id = insert(&self.pool, grant).await?;
        Ok(Grant {
            id,
            ..grant.clone()
        })
    }

    #[instrument(skip_all, fields(principal = %principal, scope = %scope))]
    async fn revoke_all(&self, principal: &Principal, scope: &Scope) -> DomainResult<u64> {
        delete_under_scope(&self.pool, principal, scope).await
    }

    #[instrument(skip(self))]
    async fn delete(&self, id: &str) -> DomainResult<()> {
        sqlx::query!("DELETE FROM grants WHERE id = $1", id)
            .execute(&self.pool)
            .await
            .to_domain()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::PgGrantRepository;
    use crate::domain::errors::DomainError;
    use crate::domain::ids::{AppId, OrganizationId, ProjectId, UserId};
    use crate::domain::role::RoleName;
    use crate::test_support::prelude::*;
    use scylla_auth::authz::{
        Grant, GrantRepository, ORGANIZATION_ADMIN_ROLE, ORGANIZATION_AGENT_ROLE,
        PROJECT_ADMIN_ROLE, Principal, SYSTEM_ADMIN_ROLE, Scope,
    };
    use sqlx::PgPool;

    fn role(name: &str) -> RoleName {
        RoleName::new(name).unwrap()
    }

    async fn seed_app(pool: &PgPool, org: &crate::domain::organization::Organization) -> AppId {
        let id = AppId::generate();
        sqlx::query!(
            "INSERT INTO apps (id, organization_id, name) VALUES ($1, $2, 'runner')",
            id.as_str(),
            org.id().as_str(),
        )
        .execute(pool)
        .await
        .unwrap();
        id
    }

    /// `principal_id` and `scope_id` are polymorphic, so triggers, not FK cascades, clear grants.
    #[sqlx::test(migrations = "../../migrations")]
    async fn deleting_an_organization_clears_the_grants_of_its_whole_subtree(pool: PgPool) {
        let org = seed_org(&pool, "acme").await;
        let other_org = seed_org(&pool, "globex").await;
        let project = seed_project(&pool, &org, "apollo").await;
        let other_project = seed_project(&pool, &other_org, "zeus").await;
        let user = seed_user(&pool, "alice").await;
        let app = seed_app(&pool, &org).await;

        let repo = PgGrantRepository::new(pool.clone());
        let doomed = [
            Grant::new(
                Principal::User(user.id().clone()),
                role(ORGANIZATION_ADMIN_ROLE),
                Scope::Organization(org.id().clone()),
            ),
            Grant::new(
                Principal::User(user.id().clone()),
                role(PROJECT_ADMIN_ROLE),
                Scope::Project(project.id().clone()),
            ),
            Grant::new(
                Principal::App(app.clone()),
                role(ORGANIZATION_AGENT_ROLE),
                Scope::Organization(org.id().clone()),
            ),
        ];
        let survivors = [
            Grant::new(
                Principal::User(user.id().clone()),
                role(ORGANIZATION_ADMIN_ROLE),
                Scope::Organization(other_org.id().clone()),
            ),
            Grant::new(
                Principal::User(user.id().clone()),
                role(PROJECT_ADMIN_ROLE),
                Scope::Project(other_project.id().clone()),
            ),
        ];
        for g in doomed.iter().chain(survivors.iter()) {
            repo.create(g).await.unwrap();
        }

        sqlx::query!("DELETE FROM organizations WHERE id = $1", org.id().as_str())
            .execute(&pool)
            .await
            .unwrap();

        let remaining = repo.list_all().await.unwrap();
        for g in &doomed {
            assert!(
                !remaining.iter().any(|r| r.id == g.id),
                "grant {} should have gone with the organization",
                g.id,
            );
        }
        for g in &survivors {
            assert!(
                remaining.iter().any(|r| r.id == g.id),
                "grant {} belongs to another org and must survive",
                g.id,
            );
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn deleting_a_user_clears_their_grants_at_every_scope(pool: PgPool) {
        let org = seed_org(&pool, "acme").await;
        let user = seed_user(&pool, "alice").await;
        let keeper = seed_user(&pool, "bob").await;

        let repo = PgGrantRepository::new(pool.clone());
        let system = Grant::new(
            Principal::User(user.id().clone()),
            role(SYSTEM_ADMIN_ROLE),
            Scope::System,
        );
        let scoped = Grant::new(
            Principal::User(user.id().clone()),
            role(ORGANIZATION_ADMIN_ROLE),
            Scope::Organization(org.id().clone()),
        );
        let others = Grant::new(
            Principal::User(keeper.id().clone()),
            role(ORGANIZATION_ADMIN_ROLE),
            Scope::Organization(org.id().clone()),
        );
        for g in [&system, &scoped, &others] {
            repo.create(g).await.unwrap();
        }

        sqlx::query!("DELETE FROM users WHERE id = $1", user.id().as_str())
            .execute(&pool)
            .await
            .unwrap();

        let remaining = repo.list_all().await.unwrap();
        assert_eq!(
            remaining.len(),
            1,
            "only the other user's grant should be left",
        );
        assert_eq!(remaining[0].id, others.id);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_repeated_grant_returns_the_stored_one(pool: PgPool) {
        let org = seed_org(&pool, "acme").await;
        let user = seed_user(&pool, "alice").await;
        let repo = PgGrantRepository::new(pool.clone());
        let grant = || {
            Grant::new(
                Principal::User(user.id().clone()),
                role(ORGANIZATION_ADMIN_ROLE),
                Scope::Organization(org.id().clone()),
            )
        };

        let first = repo.create(&grant()).await.unwrap();
        let again = repo.create(&grant()).await.unwrap();

        assert_eq!(again.id, first.id);
        let stored = repo.list_all().await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, first.id);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_grant_that_names_a_missing_principal_or_scope_is_refused(pool: PgPool) {
        let org = seed_org(&pool, "acme").await;
        let project = seed_project(&pool, &org, "apollo").await;
        let user = Principal::User(seed_user(&pool, "alice").await.id().clone());
        let repo = PgGrantRepository::new(pool.clone());
        let ghost_org = Scope::Organization(OrganizationId::new("ghost"));
        let ghost_project = Scope::Project(ProjectId::new("ghost"));

        for (principal, role_name, scope) in [
            (
                Principal::User(UserId::new("ghost")),
                ORGANIZATION_ADMIN_ROLE,
                Scope::Organization(org.id().clone()),
            ),
            (
                Principal::App(AppId::new("ghost")),
                ORGANIZATION_AGENT_ROLE,
                Scope::Organization(org.id().clone()),
            ),
            (user.clone(), ORGANIZATION_ADMIN_ROLE, ghost_org),
            (user.clone(), PROJECT_ADMIN_ROLE, ghost_project),
        ] {
            let err = repo
                .create(&Grant::new(principal, role(role_name), scope))
                .await
                .unwrap_err();
            assert!(matches!(err, DomainError::BusinessRule(_)), "{err:?}");
        }
        assert!(
            repo.create(&Grant::new(
                user,
                role(PROJECT_ADMIN_ROLE),
                Scope::Project(project.id().clone()),
            ))
            .await
            .is_ok()
        );
        assert_eq!(repo.list_all().await.unwrap().len(), 1);
    }
}
