//! The project's reads. Each query, its access and its output type. The strata module of the slice
//! runs it.

use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::domain::ids::{OrganizationId, ProjectId, UserId};
use crate::domain::permission::Permission;
use crate::domain::project::Project;
use crate::domain::user::User;
use scylla_extension::{Access, Describe, Query};

#[derive(Debug)]
pub struct GetProject {
    pub id: ProjectId,
}

impl Describe for GetProject {
    fn access(&self) -> Access {
        Access::Requires(Permission::ReadProject(self.id.clone()))
    }
}

impl Query for GetProject {
    type Output = Project;
}

#[derive(Debug)]
pub struct ListProjects {
    pub pagination: Option<PaginationParams>,
}

impl Describe for ListProjects {
    fn access(&self) -> Access {
        Access::Requires(Permission::ListProjects)
    }
}

impl Query for ListProjects {
    type Output = PaginatedResult<Project>;
}

/// Gated on `readOrganization`, not on `listProjectsByOrganization`: a project-only role must
/// see its own project, not be refused. The wider permission only widens the visible set, read
/// from the grants and not asked of the access model, so it writes no audit row.
#[derive(Debug)]
pub struct ListOrganizationProjects {
    pub organization_id: OrganizationId,
    pub pagination: Option<PaginationParams>,
}

impl Describe for ListOrganizationProjects {
    fn access(&self) -> Access {
        Access::Requires(Permission::ReadOrganization(self.organization_id.clone()))
    }
}

impl Query for ListOrganizationProjects {
    type Output = PaginatedResult<Project>;
}

#[derive(Debug)]
pub struct ListProjectMembers {
    pub project_id: ProjectId,
    pub pagination: Option<PaginationParams>,
}

impl Describe for ListProjectMembers {
    fn access(&self) -> Access {
        Access::Requires(Permission::ListProjectMembers(self.project_id.clone()))
    }
}

impl Query for ListProjectMembers {
    type Output = PaginatedResult<User>;
}

#[derive(Debug)]
pub struct ListUserProjects {
    pub user_id: UserId,
    pub pagination: Option<PaginationParams>,
}

impl Describe for ListUserProjects {
    fn access(&self) -> Access {
        Access::Requires(Permission::ListUserProjects(self.user_id.clone()))
    }
}

impl Query for ListUserProjects {
    type Output = PaginatedResult<Project>;
}
