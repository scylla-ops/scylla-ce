//! The role's writes. One block per command, in the order it runs: the struct, its access,
//! its payload types, what `Prepare` checks and builds, what `Persist` writes. A command on one
//! role is checked on the role: the access model takes the manage-roles action of its owner.

use super::RoleUseCases;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::OrganizationId;
use crate::domain::permission::Permission;
use crate::domain::role::{RoleDescription, RoleDisplayName, RoleName};
use async_trait::async_trait;
use scylla_auth::authz::{ROLE_IN_USE, Role, RoleKind, ScopeKind, validate_role_permissions};
use scylla_extension::{
    Access, Authorized, Command, Committed, Deleted, Describe, Draft, Persist, Prepare, Prepared,
    Run,
};

/// With an organization, a role of that organization; without, a platform role.
#[derive(Debug)]
pub struct CreateRole {
    pub organization_id: Option<OrganizationId>,
    pub name: RoleDisplayName,
    pub description: RoleDescription,
    pub scope: ScopeKind,
    pub kind: RoleKind,
    pub permissions: Vec<String>,
}

impl Describe for CreateRole {
    fn access(&self) -> Access {
        Access::Requires(
            self.organization_id
                .as_ref()
                .map_or(Permission::ManageRoles, |organization| {
                    Permission::ManageOrgRoles(organization.clone())
                }),
        )
    }
}

impl Command for CreateRole {
    type Staged = Draft<Role>;
    type Committed = Role;
}

#[async_trait]
impl Run<Prepare<CreateRole>> for RoleUseCases {
    async fn run(&self, input: Authorized<CreateRole>) -> DomainResult<Prepared<CreateRole>> {
        let cmd = input.command();
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
        self.ensure_no_escalation(
            input.caller(),
            &cmd.permissions,
            cmd.organization_id.as_ref(),
        )
        .await?;
        let role = Role::new_custom(
            cmd.name.clone(),
            cmd.description.clone(),
            cmd.scope,
            cmd.kind,
            cmd.organization_id.clone(),
            cmd.permissions.clone(),
        );
        Ok(input.prepared(Draft::new(role)))
    }
}

#[async_trait]
impl Run<Persist<CreateRole>> for RoleUseCases {
    async fn run(&self, input: Prepared<CreateRole>) -> DomainResult<Committed<CreateRole>> {
        input
            .commit(async |draft| {
                let role = draft.into_inner();
                self.role_repo.create(&role).await?;
                Ok(role)
            })
            .await
    }
}

#[derive(Debug)]
pub struct UpdateRole {
    pub id: RoleName,
    pub name: RoleDisplayName,
    pub description: RoleDescription,
    pub permissions: Vec<String>,
}

impl Describe for UpdateRole {
    fn access(&self) -> Access {
        Access::Requires(Permission::ManageRole(self.id.clone()))
    }
}

impl Command for UpdateRole {
    type Staged = Draft<Role>;
    type Committed = Role;
}

#[async_trait]
impl Run<Prepare<UpdateRole>> for RoleUseCases {
    async fn run(&self, input: Authorized<UpdateRole>) -> DomainResult<Prepared<UpdateRole>> {
        let cmd = input.command();
        let mut role = self.role(&cmd.id).await?;
        validate_role_permissions(&cmd.permissions, role.scope)?;
        // An edit confers only what it adds: an author may keep or drop a permission it lacks.
        let added: Vec<String> = if role.is_full_control() {
            Vec::new()
        } else {
            cmd.permissions
                .iter()
                .filter(|p| !role.permissions.contains(p))
                .cloned()
                .collect()
        };
        self.ensure_no_escalation(input.caller(), &added, role.owner_org.as_ref())
            .await?;
        role.name = cmd.name.clone();
        role.description = cmd.description.clone();
        role.permissions.clone_from(&cmd.permissions);
        Ok(input.prepared(Draft::new(role)))
    }
}

#[async_trait]
impl Run<Persist<UpdateRole>> for RoleUseCases {
    async fn run(&self, input: Prepared<UpdateRole>) -> DomainResult<Committed<UpdateRole>> {
        input
            .commit(async |draft| {
                let role = draft.into_inner();
                self.role_repo.update(&role).await?;
                Ok(role)
            })
            .await
    }
}

#[derive(Debug)]
pub struct DeleteRole {
    pub id: RoleName,
}

impl Describe for DeleteRole {
    fn access(&self) -> Access {
        Access::Requires(Permission::ManageRole(self.id.clone()))
    }
}

impl Command for DeleteRole {
    type Staged = Role;
    type Committed = Deleted<Role>;
}

#[async_trait]
impl Run<Prepare<DeleteRole>> for RoleUseCases {
    async fn run(&self, input: Authorized<DeleteRole>) -> DomainResult<Prepared<DeleteRole>> {
        let role = self.role(&input.command().id).await?;
        if role.builtin {
            return Err(DomainError::business_rule(
                "builtin roles cannot be deleted",
            ));
        }
        if self.role_repo.in_use(&role.id).await? {
            return Err(DomainError::business_rule(ROLE_IN_USE));
        }
        Ok(input.prepared(role))
    }
}

#[async_trait]
impl Run<Persist<DeleteRole>> for RoleUseCases {
    async fn run(&self, input: Prepared<DeleteRole>) -> DomainResult<Committed<DeleteRole>> {
        input
            .commit(async |role| {
                self.role_repo.delete(&role).await?;
                Ok(Deleted::new(role))
            })
            .await
    }
}
