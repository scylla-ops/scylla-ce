use crate::domain::ids::OrganizationId;
use crate::domain::organization::OrganizationName;
use crate::domain::project::ProjectName;
use crate::domain::role::{RoleDisplayName, RoleName};
use scylla_auth::authz::Scope;

/// One grant of a user or an app, with the names to show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrincipalAccess {
    pub grant_id: String,
    pub scope: Scope,
    /// The organization of an organization or a project scope.
    pub organization_id: Option<OrganizationId>,
    pub organization_name: Option<OrganizationName>,
    pub project_name: Option<ProjectName>,
    pub role: RoleName,
    pub role_name: RoleDisplayName,
}
