use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::OrganizationId;
use crate::domain::permission::permission_resource_type;
use crate::domain::role::{RoleDescription, RoleDisplayName};
use async_trait::async_trait;
use scylla_auth::authz::{FULL_CONTROL, ROLE_IN_USE, Role, RoleKind, RoleRepository, ScopeKind};
use sqlx::{PgConnection, PgPool};
use tracing::instrument;

use super::error::{DbFieldExt, SqlxResultExt};
use super::version::{from_db, to_db, written};

const SCOPE_SYSTEM: &str = "system";
const SCOPE_ORGANIZATION: &str = "organization";
const SCOPE_PROJECT: &str = "project";

#[derive(Clone)]
pub struct PgRoleRepository {
    pool: PgPool,
}

impl PgRoleRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn select(&self, id: Option<&str>) -> DomainResult<Vec<Role>> {
        let rows = sqlx::query!(
            "SELECT r.id, r.key, r.name, r.description, r.scope_kind, r.kind, r.owner_org_id, \
             r.builtin, r.version, \
             COALESCE(ARRAY_AGG(rp.permission) FILTER (WHERE rp.permission IS NOT NULL), ARRAY[]::text[]) \
                 AS \"permissions!\" \
             FROM roles r \
             LEFT JOIN role_permissions rp ON rp.role_id = r.id \
             WHERE $1::text IS NULL OR r.id = $1 \
             GROUP BY r.id",
            id,
        )
        .fetch_all(&self.pool)
        .await
        .to_domain()?;

        rows.into_iter()
            .map(|r| {
                let scope = match r.scope_kind.as_str() {
                    SCOPE_SYSTEM => ScopeKind::System,
                    SCOPE_ORGANIZATION => ScopeKind::Organization,
                    SCOPE_PROJECT => ScopeKind::Project,
                    other => {
                        return Err(DomainError::Infrastructure(format!(
                            "unknown role scope_kind '{other}'"
                        )));
                    }
                };
                let kind = RoleKind::parse(&r.kind).ok_or_else(|| {
                    DomainError::Infrastructure(format!("unknown role kind '{}'", r.kind))
                })?;
                Ok(Role {
                    id: r.id,
                    key: r.key,
                    name: RoleDisplayName::new(r.name).db_field("role name")?,
                    description: RoleDescription::new(r.description)
                        .db_field("role description")?,
                    scope,
                    kind,
                    owner_org: r.owner_org_id.map(OrganizationId::new),
                    builtin: r.builtin,
                    permissions: known_permissions(r.permissions),
                    version: from_db(r.version),
                })
            })
            .collect()
    }
}

/// A key that the code no longer knows stays in its row but gives nothing: the role keeps its
/// other permissions, and its next update drops the key.
fn known_permissions(keys: Vec<String>) -> Vec<String> {
    keys.into_iter()
        .filter(|key| key == FULL_CONTROL || permission_resource_type(key).is_some())
        .collect()
}

impl PgRoleRepository {
    async fn found(&self, id: &str) -> DomainResult<Role> {
        self.select(Some(id))
            .await?
            .pop()
            .ok_or_else(|| DomainError::not_found("Role", id))
    }
}

async fn insert_permissions(tx: &mut PgConnection, role: &Role) -> DomainResult<()> {
    sqlx::query!(
        "INSERT INTO role_permissions (role_id, permission) SELECT $1, unnest($2::text[])",
        role.id,
        &role.permissions,
    )
    .execute(tx)
    .await
    .to_domain()?;
    Ok(())
}

#[async_trait]
impl RoleRepository for PgRoleRepository {
    #[instrument(skip(self))]
    async fn list_all(&self) -> DomainResult<Vec<Role>> {
        self.select(None).await
    }

    #[instrument(skip(self))]
    async fn get(&self, id: &str) -> DomainResult<Option<Role>> {
        Ok(self.select(Some(id)).await?.pop())
    }

    #[instrument(skip_all, fields(role_id = %role.id))]
    async fn create(&self, role: &Role) -> DomainResult<()> {
        let mut tx = self.pool.begin().await.to_domain()?;
        sqlx::query!(
            "INSERT INTO roles (id, key, name, description, scope_kind, kind, owner_org_id, builtin) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            role.id,
            role.key.as_deref(),
            role.name.as_str(),
            role.description.as_str(),
            role.scope.as_str(),
            role.kind.as_str(),
            role.owner_org.as_ref().map(OrganizationId::as_str),
            role.builtin,
        )
        .execute(&mut *tx)
        .await
        .to_domain()?;
        insert_permissions(&mut tx, role).await?;
        tx.commit().await.to_domain()
    }

    #[instrument(skip_all, fields(role_id = %role.id))]
    async fn update(&self, role: &Role) -> DomainResult<()> {
        let mut tx = self.pool.begin().await.to_domain()?;
        let updated = sqlx::query!(
            "UPDATE roles SET name = $2, description = $3, updated_at = NOW(), version = version + 1 \
             WHERE id = $1 AND version = $4",
            role.id,
            role.name.as_str(),
            role.description.as_str(),
            to_db(role.version),
        )
        .execute(&mut *tx)
        .await
        .to_domain()?
        .rows_affected()
            > 0;
        if updated {
            sqlx::query!("DELETE FROM role_permissions WHERE role_id = $1", role.id)
                .execute(&mut *tx)
                .await
                .to_domain()?;
            insert_permissions(&mut tx, role).await?;
        }
        tx.commit().await.to_domain()?;
        written(
            updated.then_some(()),
            "Role",
            &role.id,
            self.found(&role.id),
        )
        .await
    }

    #[instrument(skip_all, fields(role_id = %role.id, version = role.version))]
    async fn delete(&self, role: &Role) -> DomainResult<()> {
        let mut tx = self.pool.begin().await.to_domain()?;
        // FOR UPDATE waits for a grant that is being written with this role, and holds back the
        // next one, so the use check below cannot go stale.
        let version = sqlx::query_scalar!(
            "SELECT version FROM roles WHERE id = $1 FOR UPDATE",
            role.id,
        )
        .fetch_optional(&mut *tx)
        .await
        .to_domain()?
        .ok_or_else(|| DomainError::not_found("Role", &role.id))?;
        if version != to_db(role.version) {
            return Err(DomainError::stale("Role", &role.id));
        }
        if used(&mut *tx, &role.id).await? {
            return Err(DomainError::business_rule(ROLE_IN_USE));
        }
        sqlx::query!("DELETE FROM roles WHERE id = $1", role.id)
            .execute(&mut *tx)
            .await
            .to_domain()?;
        tx.commit().await.to_domain()
    }

    #[instrument(skip(self))]
    async fn in_use(&self, id: &str) -> DomainResult<bool> {
        used(&self.pool, id).await
    }
}

async fn used<'e>(executor: impl sqlx::PgExecutor<'e>, id: &str) -> DomainResult<bool> {
    sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM grants WHERE role_id = $1) AS \"used!\"",
        id,
    )
    .fetch_one(executor)
    .await
    .to_domain()
}

#[cfg(test)]
mod tests {
    use super::PgRoleRepository;
    use crate::domain::errors::DomainError;
    use crate::domain::role::{RoleDescription, RoleDisplayName, RoleName};
    use scylla_auth::authz::{Role, RoleKind, RoleRepository, ScopeKind};
    use sqlx::PgPool;

    #[sqlx::test(migrations = "../../migrations")]
    async fn an_organization_viewer_lists_the_members_of_its_projects(pool: PgPool) {
        let viewer = PgRoleRepository::new(pool)
            .get(scylla_auth::authz::ORGANIZATION_VIEWER_ROLE)
            .await
            .unwrap()
            .unwrap();
        assert!(
            viewer
                .permissions
                .contains(&"listProjectMembers".to_string())
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_key_that_the_code_does_not_know_is_dropped_and_the_role_keeps_working(pool: PgPool) {
        use crate::domain::caller::CallerContext;
        use crate::domain::permission::Permission;
        use crate::postgres::{PgAuthzEntityProvider, PgGrantRepository};
        use crate::test_support::prelude::*;
        use scylla_auth::audit::NoopAuditLog;
        use scylla_auth::authz::{Grant, GrantRepository, PermissionService, Principal, Scope};
        use scylla_auth::cedar::CedarPermissionService;
        use std::sync::Arc;

        let org = seed_org(&pool, "legacy").await;
        let user = seed_user(&pool, "erin").await;
        sqlx::query(
            "INSERT INTO roles (id, name, scope_kind) VALUES ('legacy', 'Legacy', 'organization')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO role_permissions (role_id, permission) VALUES \
             ('legacy', 'retiredPermission'), ('legacy', 'readOrganization')",
        )
        .execute(&pool)
        .await
        .unwrap();
        PgGrantRepository::new(pool.clone())
            .create(&Grant::new(
                Principal::User(user.id().clone()),
                RoleName::new("legacy").unwrap(),
                Scope::Organization(org.id().clone()),
            ))
            .await
            .unwrap();

        let roles = PgRoleRepository::new(pool.clone());
        let role = roles.get("legacy").await.unwrap().unwrap();
        assert_eq!(role.permissions, ["readOrganization"]);

        let permissions = CedarPermissionService::new(
            Arc::new(PgAuthzEntityProvider::new(pool.clone())),
            Arc::new(roles),
            Arc::new(PgGrantRepository::new(pool)),
            Arc::new(NoopAuditLog),
        )
        .await
        .unwrap();
        permissions
            .check(
                &CallerContext::User(user.id().clone()),
                Permission::ReadOrganization(org.id().clone()),
            )
            .await
            .expect("the known permission of the role still applies");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn role_crud_round_trip(pool: PgPool) {
        let repo = PgRoleRepository::new(pool);
        let role = Role::new_custom(
            RoleDisplayName::new("CI Runner").unwrap(),
            RoleDescription::new("reads and runs pipelines").unwrap(),
            ScopeKind::Project,
            RoleKind::Member,
            None,
            vec!["readPipeline".into(), "runPipeline".into()],
        );
        let id = role.id.clone();
        repo.create(&role).await.unwrap();

        let got = repo
            .get(&id)
            .await
            .unwrap()
            .expect("role exists after create");
        assert_eq!(got.name.as_str(), "CI Runner");
        assert_eq!(got.permissions.len(), 2);
        assert!(!got.builtin);
        assert!(got.key.is_none());

        let mut updated = got.clone();
        updated.name = RoleDisplayName::new("CI").unwrap();
        updated.permissions = vec!["readPipeline".into()];
        repo.update(&updated).await.unwrap();
        let got = repo.get(&id).await.unwrap().unwrap();
        assert_eq!(got.name.as_str(), "CI");
        assert_eq!(got.permissions, vec!["readPipeline".to_string()]);
        assert_eq!(got.version, 1);

        let stale = repo.update(&updated).await.unwrap_err();
        assert!(matches!(stale, DomainError::Stale(_)), "{stale}");
        assert!(matches!(
            repo.delete(&updated).await.unwrap_err(),
            DomainError::Stale(_)
        ));

        repo.delete(&got).await.unwrap();
        assert!(repo.get(&id).await.unwrap().is_none());
    }

    fn owned(name: &str, organization: &crate::domain::organization::Organization) -> Role {
        Role::new_custom(
            RoleDisplayName::new(name).unwrap(),
            RoleDescription::new("").unwrap(),
            ScopeKind::Project,
            RoleKind::Member,
            Some(organization.id().clone()),
            vec!["runPipeline".into()],
        )
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_role_name_is_unique_per_owner_without_case_and_never_a_platform_name(pool: PgPool) {
        use crate::test_support::prelude::*;
        let o1 = seed_org(&pool, "o1").await;
        let o2 = seed_org(&pool, "o2").await;
        let repo = PgRoleRepository::new(pool);
        let conflict = |err: DomainError| {
            assert!(
                matches!(&err, DomainError::Conflict(m) if m == "Role name already exists"),
                "{err}"
            );
        };

        repo.create(&owned("Deployer", &o1)).await.unwrap();
        repo.create(&owned("Deployer", &o2)).await.unwrap();
        conflict(repo.create(&owned("Deployer", &o1)).await.unwrap_err());
        conflict(repo.create(&owned("deployer", &o1)).await.unwrap_err());
        conflict(
            repo.create(&owned("project VIEWER", &o1))
                .await
                .unwrap_err(),
        );
        let platform = Role::new_custom(
            RoleDisplayName::new("Deployer").unwrap(),
            RoleDescription::new("").unwrap(),
            ScopeKind::Project,
            RoleKind::Member,
            None,
            vec!["runPipeline".into()],
        );
        repo.create(&platform)
            .await
            .expect("an organization does not block a platform name");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_statement_that_changes_a_given_role_or_grant_is_checked(pool: PgPool) {
        use crate::postgres::PgGrantRepository;
        use crate::test_support::prelude::*;
        use scylla_auth::authz::{Grant, GrantRepository, Principal, Scope};
        let (o1, project, _) = seed_org_project_pipeline(&pool, "bound").await;
        let o2 = seed_org(&pool, "other").await;
        let user = seed_user(&pool, "erin").await;
        let repo = PgRoleRepository::new(pool.clone());
        let mine = owned("Deployer", &o1);
        let theirs = owned("Deployer", &o2);
        repo.create(&mine).await.unwrap();
        repo.create(&theirs).await.unwrap();
        PgGrantRepository::new(pool.clone())
            .create(&Grant::new(
                Principal::User(user.id().clone()),
                RoleName::new(&mine.id).unwrap(),
                Scope::Project(project.id().clone()),
            ))
            .await
            .unwrap();
        let refused = |sql: &'static str, bind: String| {
            let pool = pool.clone();
            async move {
                sqlx::query(sql)
                    .bind(bind)
                    .execute(&pool)
                    .await
                    .expect_err(sql)
            }
        };

        refused("UPDATE grants SET role_id = $1", theirs.id.clone()).await;
        refused(
            "UPDATE roles SET kind = 'agent' WHERE id = $1",
            mine.id.clone(),
        )
        .await;
        refused(
            "UPDATE roles SET scope_kind = 'organization' WHERE id = $1",
            mine.id.clone(),
        )
        .await;
        sqlx::query("UPDATE roles SET description = 'still fine' WHERE id = $1")
            .bind(&mine.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_role_is_in_use_while_granted(pool: PgPool) {
        use crate::postgres::PgGrantRepository;
        use crate::test_support::prelude::*;
        use scylla_auth::authz::{Grant, GrantRepository, Principal, Scope};
        let (org, project, _) = seed_org_project_pipeline(&pool, "used").await;
        let user = seed_user(&pool, "carol").await;
        let repo = PgRoleRepository::new(pool.clone());
        let role = owned("Deployer", &org);
        repo.create(&role).await.unwrap();
        assert!(!repo.in_use(&role.id).await.unwrap());

        PgGrantRepository::new(pool)
            .create(&Grant::new(
                Principal::User(user.id().clone()),
                RoleName::new(&role.id).unwrap(),
                Scope::Project(project.id().clone()),
            ))
            .await
            .unwrap();
        assert!(repo.in_use(&role.id).await.unwrap());

        let err = repo.delete(&role).await.unwrap_err();
        assert!(matches!(err, DomainError::BusinessRule(_)));
        assert!(repo.get(&role.id).await.unwrap().is_some());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn an_organization_delete_takes_its_roles_and_their_grants(pool: PgPool) {
        use crate::postgres::{PgGrantRepository, PgOrganizationRepository};
        use crate::test_support::prelude::*;
        use scylla_auth::authz::{Grant, GrantRepository, Principal, Scope};
        use scylla_core::application::organization::OrganizationRepository;
        let (org, project, _) = seed_org_project_pipeline(&pool, "gone").await;
        let other = seed_org(&pool, "stays").await;
        let user = seed_user(&pool, "carol").await;
        let repo = PgRoleRepository::new(pool.clone());
        let role = owned("Deployer", &org);
        let kept = owned("Deployer", &other);
        repo.create(&role).await.unwrap();
        repo.create(&kept).await.unwrap();
        PgGrantRepository::new(pool.clone())
            .create(&Grant::new(
                Principal::User(user.id().clone()),
                RoleName::new(&role.id).unwrap(),
                Scope::Project(project.id().clone()),
            ))
            .await
            .unwrap();

        PgOrganizationRepository::new(pool.clone())
            .delete(&org)
            .await
            .unwrap();

        assert!(repo.get(&role.id).await.unwrap().is_none());
        assert!(repo.get(&kept.id).await.unwrap().is_some());
        let grants: i64 = sqlx::query_scalar("SELECT count(*) FROM grants WHERE role_id = $1")
            .bind(&role.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(grants, 0);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_display_name_is_unique_across_roles(pool: PgPool) {
        let repo = PgRoleRepository::new(pool);
        let shadow = Role::new_custom(
            RoleDisplayName::new("System Admin").unwrap(),
            RoleDescription::new("").unwrap(),
            ScopeKind::Project,
            RoleKind::Member,
            None,
            vec!["readPipeline".into()],
        );
        let err = repo.create(&shadow).await.unwrap_err();
        assert!(
            matches!(&err, DomainError::Conflict(m) if m == "Role name already exists"),
            "{err}"
        );
        let lower = Role::new_custom(
            RoleDisplayName::new("system admin").unwrap(),
            RoleDescription::new("").unwrap(),
            ScopeKind::Project,
            RoleKind::Member,
            None,
            vec!["readPipeline".into()],
        );
        assert!(matches!(
            repo.create(&lower).await.unwrap_err(),
            DomainError::Conflict(_)
        ));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn effective_permissions_resolves_roles_and_direct_grants(pool: PgPool) {
        use crate::domain::caller::{CallerContext, ServiceIdentity};
        use crate::postgres::PgGrantRepository;
        use crate::test_support::prelude::*;
        use scylla_auth::authz::{Grant, GrantRepository, Principal, Scope};
        use scylla_core::application::role::{GetEffectivePermissions, RoleUseCases};
        use scylla_core::test_support::authz::{RecordingPermissionService, actions};
        use std::sync::Arc;

        sqlx::query!(
            "INSERT INTO roles (id, name, scope_kind, builtin) VALUES ('ci', 'CI', 'project', FALSE)"
        )
        .execute(&pool)
        .await
        .unwrap();
        for p in ["readPipeline", "runPipeline"] {
            sqlx::query!(
                "INSERT INTO role_permissions (role_id, permission) VALUES ('ci', $1)",
                p
            )
            .execute(&pool)
            .await
            .unwrap();
        }
        sqlx::query!(
            "INSERT INTO roles (id, name, scope_kind, builtin) \
             VALUES ('janitor', 'Janitor', 'organization', FALSE)"
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!(
            "INSERT INTO role_permissions (role_id, permission) VALUES ('janitor', 'deleteJob')"
        )
        .execute(&pool)
        .await
        .unwrap();
        let org = seed_org(&pool, "o1").await;
        let project = seed_project(&pool, &org, "p1").await;
        let alice = seed_user(&pool, "alice").await;
        let grants = PgGrantRepository::new(pool.clone());
        for (role, scope) in [
            ("ci", Scope::Project(project.id().clone())),
            ("janitor", Scope::Organization(org.id().clone())),
        ] {
            grants
                .create(&Grant::new(
                    Principal::User(alice.id().clone()),
                    RoleName::new(role).unwrap(),
                    scope,
                ))
                .await
                .unwrap();
        }

        let uc = RoleUseCases::new(
            Arc::new(PgRoleRepository::new(pool.clone())),
            Arc::new(PgGrantRepository::new(pool)),
        );
        let actions = actions(Arc::new(RecordingPermissionService::new()));
        let caller = CallerContext::Service(ServiceIdentity::recorder());
        let scopes = actions
            .run(
                &uc,
                &caller,
                GetEffectivePermissions {
                    principal: Principal::User(alice.id().clone()),
                },
            )
            .await
            .unwrap();

        assert_eq!(scopes.len(), 2);
        let on_project = scopes
            .iter()
            .find(|s| matches!(&s.scope, Scope::Project(p) if p == project.id()))
            .expect("project p1 scope");
        assert!(!on_project.full_control);
        assert!(on_project.permissions.contains(&"readPipeline".to_string()));
        assert!(on_project.permissions.contains(&"runPipeline".to_string()));
        let on_org = scopes
            .iter()
            .find(|s| matches!(&s.scope, Scope::Organization(o) if o == org.id()))
            .expect("org o1 scope");
        assert_eq!(on_org.permissions, vec!["deleteJob".to_string()]);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn my_permissions_needs_no_permission_unlike_the_admin_view(pool: PgPool) {
        use crate::domain::caller::{CallerContext, ServiceIdentity};
        use crate::domain::ids::UserId;
        use crate::postgres::PgGrantRepository;
        use crate::test_support::prelude::*;
        use scylla_auth::authz::{
            Grant, GrantRepository, ORGANIZATION_ADMIN_ROLE, Principal, Scope,
        };
        use scylla_core::application::role::{
            GetEffectivePermissions, GetMyPermissions, RoleUseCases,
        };
        use scylla_core::test_support::authz::{DenyingPermissionService, actions};
        use std::sync::Arc;

        let org = seed_org(&pool, "o1").await;
        let alice = seed_user(&pool, "alice").await;
        PgGrantRepository::new(pool.clone())
            .create(&Grant::new(
                Principal::User(alice.id().clone()),
                RoleName::new(ORGANIZATION_ADMIN_ROLE).unwrap(),
                Scope::Organization(org.id().clone()),
            ))
            .await
            .unwrap();

        let uc = RoleUseCases::new(
            Arc::new(PgRoleRepository::new(pool.clone())),
            Arc::new(PgGrantRepository::new(pool)),
        );
        let actions = actions(Arc::new(DenyingPermissionService::new()));

        let alice = CallerContext::User(alice.id().clone());
        let scopes = actions
            .run(&uc, &alice, GetMyPermissions)
            .await
            .expect("own permissions");
        assert_eq!(scopes.len(), 1);
        assert!(matches!(&scopes[0].scope, Scope::Organization(o) if o == org.id()));
        assert!(scopes[0].full_control, "organization-admin confers '*'");

        let bob = CallerContext::User(UserId::new("bob"));
        let scopes = actions.run(&uc, &bob, GetMyPermissions).await.unwrap();
        assert!(scopes.is_empty());

        assert!(
            actions
                .run(
                    &uc,
                    &alice,
                    GetEffectivePermissions {
                        principal: Principal::User(UserId::new("bob")),
                    },
                )
                .await
                .is_err(),
            "reading another principal must still require manageSystemGrants",
        );

        let service = CallerContext::Service(ServiceIdentity::recorder());
        let refused = actions.run(&uc, &service, GetMyPermissions).await;
        assert!(refused.is_err());
    }
}
