use crate::access::CallerOf;
use scylla_auth::authz::{
    EffectiveScope, FULL_CONTROL, GrantRepository, Principal, ROLE_IN_USE, Role, RoleKind,
    RoleRepository, Scope, ScopeKind, ensure_no_escalation, permissions_by_role,
    validate_role_permissions,
};
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::OrganizationId;
use scylla_domain::domain::permission::{PERMISSION_CATALOG, Permission};
use scylla_domain::domain::role::{RoleDescription, RoleDisplayName, RoleName};
use scylla_extension::Access;
use std::collections::BTreeSet;
use std::sync::Arc;

#[strata::module(section = "roles")]
pub trait Roles: crate::Access {
    #[component]
    type RoleRepo = Arc<dyn RoleRepository>;

    #[component]
    type GrantRepo = Arc<dyn GrantRepository>;

    async fn role(&self, id: &RoleName) -> DomainResult<Role> {
        self.role_repo()
            .get(id.as_str())
            .await?
            .ok_or_else(|| DomainError::not_found("Role", id))
    }

    async fn ensure_no_escalation(
        &self,
        permissions: &[String],
        organization: Option<&OrganizationId>,
    ) -> DomainResult<()> {
        let scope = organization.map_or(Scope::System, |o| Scope::Organization(o.clone()));
        let roles = self.role_repo().list_all().await?;
        let grants = self.grant_repo().list_all().await?;
        ensure_no_escalation(
            &roles,
            &grants,
            self.ctx().caller(),
            permissions,
            &scope,
            organization,
        )
    }

    async fn effective_scopes(&self, principal: &Principal) -> DomainResult<Vec<EffectiveScope>> {
        let role_perms = permissions_by_role(self.role_repo().as_ref()).await?;
        let grants = self.grant_repo().list_all().await?;
        let mut groups: Vec<(Scope, BTreeSet<String>)> = Vec::new();
        for grant in grants.iter().filter(|g| g.principal == *principal) {
            let perms: Vec<String> = role_perms
                .get(grant.role.as_str())
                .cloned()
                .unwrap_or_default();
            if let Some(idx) = groups.iter().position(|(s, _)| *s == grant.scope) {
                groups[idx].1.extend(perms);
            } else {
                groups.push((grant.scope.clone(), perms.into_iter().collect()));
            }
        }
        Ok(groups
            .into_iter()
            .map(|(scope, set)| {
                let full_control = set.contains(FULL_CONTROL);
                EffectiveScope {
                    scope,
                    full_control,
                    permissions: if full_control {
                        Vec::new()
                    } else {
                        set.into_iter().collect()
                    },
                }
            })
            .collect())
    }

    /// With an organization, a role of that organization; without, a platform role.
    #[usecase(write, access = Access::Requires(organization_id.as_ref().map_or(Permission::ManageRoles, |o| Permission::ManageOrgRoles(o.clone()))))]
    async fn create_role(
        &self,
        organization_id: Option<OrganizationId>,
        name: RoleDisplayName,
        description: RoleDescription,
        scope: ScopeKind,
        kind: RoleKind,
        permissions: Vec<String>,
    ) -> DomainResult<Role> {
        if organization_id.is_some() && scope == ScopeKind::System {
            return Err(DomainError::validation(
                "a role of an organization is organization or project scoped",
            ));
        }
        if kind == RoleKind::Admin {
            return Err(DomainError::validation(
                "the admin kind is reserved for the builtin roles",
            ));
        }
        validate_role_permissions(&permissions, scope)?;
        self.ensure_no_escalation(&permissions, organization_id.as_ref())
            .await?;
        let role = Role::new_custom(name, description, scope, kind, organization_id, permissions);
        self.role_repo().create(&role).await?;
        Ok(role)
    }

    #[usecase(write, access = Access::Requires(Permission::ManageRole(id.clone())))]
    async fn update_role(
        &self,
        id: RoleName,
        name: RoleDisplayName,
        description: RoleDescription,
        permissions: Vec<String>,
    ) -> DomainResult<Role> {
        let mut role = self.role(&id).await?;
        validate_role_permissions(&permissions, role.scope)?;
        let added: Vec<String> = if role.is_full_control() {
            Vec::new()
        } else {
            permissions
                .iter()
                .filter(|p| !role.permissions.contains(p))
                .cloned()
                .collect()
        };
        self.ensure_no_escalation(&added, role.owner_org.as_ref())
            .await?;
        role.name = name;
        role.description = description;
        role.permissions = permissions;
        self.role_repo().update(&role).await?;
        Ok(role)
    }

    #[usecase(write, access = Access::Requires(Permission::ManageRole(id.clone())))]
    async fn delete_role(&self, id: RoleName) -> DomainResult<()> {
        let role = self.role(&id).await?;
        if role.builtin {
            return Err(DomainError::business_rule(
                "builtin roles cannot be deleted",
            ));
        }
        if self.role_repo().in_use(&role.id).await? {
            return Err(DomainError::business_rule(ROLE_IN_USE));
        }
        self.role_repo().delete(&role).await
    }

    /// With an organization, the platform roles and the roles of that organization; without, every
    /// role.
    #[usecase(read, access = Access::Requires(organization_id.as_ref().map_or(Permission::ManageRoles, |o| Permission::ManageOrgRoles(o.clone()))))]
    async fn list_roles(&self, organization_id: Option<OrganizationId>) -> DomainResult<Vec<Role>> {
        let mut roles = self.role_repo().list_all().await?;
        if let Some(organization) = &organization_id {
            roles.retain(|r| r.usable_in(Some(organization)));
        }
        Ok(roles)
    }

    #[usecase(read, access = Access::Requires(Permission::ManageRole(id.clone())))]
    async fn get_role(&self, id: RoleName) -> DomainResult<Role> {
        self.role(&id).await
    }

    /// Each permission key with the resource type that it targets. It is compiled-in data, thus
    /// each signed-in caller reads it.
    #[usecase(read, access = Access::Authenticated)]
    async fn list_authz_vocabulary(&self) -> DomainResult<&'static [(&'static str, &'static str)]> {
        Ok(PERMISSION_CATALOG.as_slice())
    }

    /// The permissions of another principal: the admin view. A caller reads its own permissions
    /// with `get_my_permissions`.
    #[usecase(read, access = Access::Requires(Permission::ManageSystemGrants))]
    async fn get_effective_permissions(
        &self,
        principal: Principal,
    ) -> DomainResult<Vec<EffectiveScope>> {
        self.effective_scopes(&principal).await
    }

    /// The permissions of the caller: no permission is asked. The use case refuses a service, not
    /// the authorize stage: a service holds no grants, and an empty list reads as "no permissions".
    #[usecase(read, access = Access::Authenticated)]
    async fn get_my_permissions(&self) -> DomainResult<Vec<EffectiveScope>> {
        let principal = Principal::from_caller(self.ctx().caller()).ok_or_else(|| {
            DomainError::forbidden("this caller is not a principal that holds grants")
        })?;
        self.effective_scopes(&principal).await
    }
}
