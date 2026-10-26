//! The organization's reads. Each query, its access and its output type. The strata module of the
//! slice runs it.

use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::domain::ids::{OrganizationId, UserId};
use crate::domain::organization::Organization;
use crate::domain::permission::Permission;
use crate::domain::user::User;
use scylla_extension::{Access, Describe, Query};

#[derive(Debug)]
pub struct GetOrganization {
    pub id: OrganizationId,
}

impl Describe for GetOrganization {
    fn access(&self) -> Access {
        Access::Requires(Permission::ReadOrganization(self.id.clone()))
    }
}

impl Query for GetOrganization {
    type Output = Organization;
}

#[derive(Debug)]
pub struct ListOrganizations {
    pub pagination: Option<PaginationParams>,
}

impl Describe for ListOrganizations {
    fn access(&self) -> Access {
        Access::Requires(Permission::ListOrganizations)
    }
}

impl Query for ListOrganizations {
    type Output = PaginatedResult<Organization>;
}

#[derive(Debug)]
pub struct ListOrganizationMembers {
    pub organization_id: OrganizationId,
    pub pagination: Option<PaginationParams>,
}

impl Describe for ListOrganizationMembers {
    fn access(&self) -> Access {
        Access::Requires(Permission::ListOrganizationMembers(
            self.organization_id.clone(),
        ))
    }
}

impl Query for ListOrganizationMembers {
    type Output = PaginatedResult<User>;
}

#[derive(Debug)]
pub struct ListUserOrganizations {
    pub user_id: UserId,
    pub pagination: Option<PaginationParams>,
}

impl Describe for ListUserOrganizations {
    fn access(&self) -> Access {
        Access::Requires(Permission::ListUserOrganizations(self.user_id.clone()))
    }
}

impl Query for ListUserOrganizations {
    type Output = PaginatedResult<Organization>;
}
