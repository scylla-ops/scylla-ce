//! The role's writes. Each command, its access and its payload types. The strata module of the
//! slice runs it. A command on one role is checked on the role: the access model takes the
//! manage-roles action of its owner.

use crate::domain::ids::OrganizationId;
use crate::domain::permission::Permission;
use crate::domain::role::{RoleDescription, RoleDisplayName, RoleName};
use scylla_auth::authz::{Role, RoleKind, ScopeKind};
use scylla_extension::{Access, Command, Deleted, Describe, Draft};

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
