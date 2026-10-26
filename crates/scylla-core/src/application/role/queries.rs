//! The role's reads. Each query, its access and its output type. The strata module of the slice
//! runs it.

use crate::domain::ids::OrganizationId;
use crate::domain::permission::Permission;
use crate::domain::role::RoleName;
use scylla_auth::authz::{EffectiveScope, Principal, Role};
use scylla_extension::{Access, Describe, Query};

/// With an organization, the platform roles and that organization's; without, every role.
#[derive(Debug)]
pub struct ListRoles {
    pub organization_id: Option<OrganizationId>,
}

impl Describe for ListRoles {
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

impl Query for ListRoles {
    type Output = Vec<Role>;
}

#[derive(Debug)]
pub struct GetRole {
    pub id: RoleName,
}

impl Describe for GetRole {
    fn access(&self) -> Access {
        Access::Requires(Permission::ManageRole(self.id.clone()))
    }
}

impl Query for GetRole {
    type Output = Role;
}

/// Every permission key with the resource type it targets: compiled-in data, so any signed-in
/// caller reads it.
#[derive(Debug)]
pub struct ListAuthzVocabulary;

impl Describe for ListAuthzVocabulary {
    fn access(&self) -> Access {
        Access::Authenticated
    }
}

impl Query for ListAuthzVocabulary {
    type Output = &'static [(&'static str, &'static str)];
}

/// Another principal's permissions, the admin view; a caller reads its own through
/// `GetMyPermissions`.
#[derive(Debug)]
pub struct GetEffectivePermissions {
    pub principal: Principal,
}

impl Describe for GetEffectivePermissions {
    fn access(&self) -> Access {
        Access::Requires(Permission::ManageSystemGrants)
    }
}

impl Query for GetEffectivePermissions {
    type Output = Vec<EffectiveScope>;
}

/// The caller's own permissions: no permission is asked. A service is refused here, not in
/// `Authorize`: it holds no grants, and an empty list would read as "no permissions".
#[derive(Debug)]
pub struct GetMyPermissions;

impl Describe for GetMyPermissions {
    fn access(&self) -> Access {
        Access::Authenticated
    }
}

impl Query for GetMyPermissions {
    type Output = Vec<EffectiveScope>;
}
