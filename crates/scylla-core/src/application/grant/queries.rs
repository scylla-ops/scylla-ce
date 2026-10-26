//! The grant's reads. Each query, its access and its output type. The strata module of the slice
//! runs it.

use crate::domain::ids::OrganizationId;
use crate::domain::permission::Permission;
use scylla_auth::authz::{Grant, Role, Scope, ScopeKind};
use scylla_extension::{Access, Describe, Query};

/// Without a scope, every grant of the installation; with one, the grants bound at that scope.
#[derive(Debug)]
pub struct ListGrants {
    pub scope: Option<Scope>,
}

impl Describe for ListGrants {
    fn access(&self) -> Access {
        Access::Requires(
            self.scope
                .as_ref()
                .map_or(Permission::ManageSystemGrants, Scope::manage_permission),
        )
    }
}

impl Query for ListGrants {
    type Output = Vec<Grant>;
}

/// The roles a grant may bind: the platform roles, and with an organization the roles of that
/// organization, which its members may read.
#[derive(Debug)]
pub struct ListGrantableRoles {
    pub scope_kind: Option<ScopeKind>,
    pub organization_id: Option<OrganizationId>,
}

impl Describe for ListGrantableRoles {
    fn access(&self) -> Access {
        match &self.organization_id {
            Some(organization) => {
                Access::Requires(Permission::ReadOrganization(organization.clone()))
            }
            None => Access::Authenticated,
        }
    }
}

impl Query for ListGrantableRoles {
    type Output = Vec<Role>;
}
