use crate::access::CallerOf;
use scylla_auth::authz::{
    EffectiveScope, FULL_CONTROL, GrantRepository, Principal, ROLE_IN_USE, Role, RoleKind,
    RoleRepository, Scope, ScopeKind, ensure_no_escalation, permissions_by_role,
    validate_role_permissions,
};
use scylla_core::application::role::{
    CreateRole, DeleteRole, GetEffectivePermissions, GetMyPermissions, GetRole,
    ListAuthzVocabulary, ListRoles, UpdateRole,
};
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::OrganizationId;
use scylla_domain::domain::permission::PERMISSION_CATALOG;
use scylla_domain::domain::role::RoleName;
use scylla_extension::Describe;
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

    #[usecase(write, access = cmd.access())]
    async fn create_role(&self, cmd: CreateRole) -> DomainResult<Role> {
        if cmd.organization_id.is_some() && cmd.scope == ScopeKind::System {
            return Err(DomainError::validation(
                "a role of an organization is organization or project scoped",
            ));
        }
        if cmd.kind == RoleKind::Admin {
            return Err(DomainError::validation(
                "the admin kind is reserved for the builtin roles",
            ));
        }
        validate_role_permissions(&cmd.permissions, cmd.scope)?;
        self.ensure_no_escalation(&cmd.permissions, cmd.organization_id.as_ref())
            .await?;
        let role = Role::new_custom(
            cmd.name,
            cmd.description,
            cmd.scope,
            cmd.kind,
            cmd.organization_id,
            cmd.permissions,
        );
        self.role_repo().create(&role).await?;
        Ok(role)
    }

    #[usecase(write, access = cmd.access())]
    async fn update_role(&self, cmd: UpdateRole) -> DomainResult<Role> {
        let mut role = self.role(&cmd.id).await?;
        validate_role_permissions(&cmd.permissions, role.scope)?;
        let added: Vec<String> = if role.is_full_control() {
            Vec::new()
        } else {
            cmd.permissions
                .iter()
                .filter(|p| !role.permissions.contains(p))
                .cloned()
                .collect()
        };
        self.ensure_no_escalation(&added, role.owner_org.as_ref())
            .await?;
        role.name = cmd.name;
        role.description = cmd.description;
        role.permissions = cmd.permissions;
        self.role_repo().update(&role).await?;
        Ok(role)
    }

    #[usecase(write, access = cmd.access())]
    async fn delete_role(&self, cmd: DeleteRole) -> DomainResult<()> {
        let role = self.role(&cmd.id).await?;
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

    #[usecase(read, access = query.access())]
    async fn list_roles(&self, query: ListRoles) -> DomainResult<Vec<Role>> {
        let mut roles = self.role_repo().list_all().await?;
        if let Some(organization) = &query.organization_id {
            roles.retain(|r| r.usable_in(Some(organization)));
        }
        Ok(roles)
    }

    #[usecase(read, access = query.access())]
    async fn get_role(&self, query: GetRole) -> DomainResult<Role> {
        self.role(&query.id).await
    }

    #[usecase(read, access = ListAuthzVocabulary.access())]
    #[allow(unused_variables)]
    async fn list_authz_vocabulary(
        &self,
        query: ListAuthzVocabulary,
    ) -> DomainResult<&'static [(&'static str, &'static str)]> {
        Ok(PERMISSION_CATALOG.as_slice())
    }

    #[usecase(read, access = query.access())]
    async fn get_effective_permissions(
        &self,
        query: GetEffectivePermissions,
    ) -> DomainResult<Vec<EffectiveScope>> {
        self.effective_scopes(&query.principal).await
    }

    #[usecase(read, access = GetMyPermissions.access())]
    #[allow(unused_variables)]
    async fn get_my_permissions(
        &self,
        query: GetMyPermissions,
    ) -> DomainResult<Vec<EffectiveScope>> {
        let principal = Principal::from_caller(self.ctx().caller()).ok_or_else(|| {
            DomainError::forbidden("this caller is not a principal that holds grants")
        })?;
        self.effective_scopes(&principal).await
    }
}
