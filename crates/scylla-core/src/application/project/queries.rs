//! The project's reads. One block per query, in the order it runs: the struct, its access
//! and path, its output type, what `Fetch` reads.

use super::ProjectUseCases;
use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::application::user::users_in_order;
use crate::domain::errors::DomainResult;
use crate::domain::ids::{OrganizationId, ProjectId, UserId};
use crate::domain::permission::Permission;
use crate::domain::project::Project;
use crate::domain::user::User;
use async_trait::async_trait;
use scylla_auth::authz::Visibility;
use scylla_extension::{Access, Authorized, Describe, Fetch, Fetched, Query, Run};

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

#[async_trait]
impl Run<Fetch<GetProject>> for ProjectUseCases {
    async fn run(&self, input: Authorized<GetProject>) -> DomainResult<Fetched<GetProject>> {
        let project = self.project_repo.find_by_id(&input.command().id).await?;
        Ok(input.fetched(project))
    }
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

#[async_trait]
impl Run<Fetch<ListProjects>> for ProjectUseCases {
    async fn run(&self, input: Authorized<ListProjects>) -> DomainResult<Fetched<ListProjects>> {
        let page = self
            .project_repo
            .list_all(input.command().pagination.as_ref())
            .await?;
        Ok(input.fetched(page))
    }
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

#[async_trait]
impl Run<Fetch<ListOrganizationProjects>> for ProjectUseCases {
    async fn run(
        &self,
        input: Authorized<ListOrganizationProjects>,
    ) -> DomainResult<Fetched<ListOrganizationProjects>> {
        let query = input.command();
        let caller = input.caller();
        let wide = Permission::ListProjectsByOrganization(query.organization_id.clone());
        let visible = match self.visibility.visible_scopes(caller, wide.key()).await? {
            Visibility::Scoped { orgs, .. } if !orgs.contains(&query.organization_id) => {
                self.visibility
                    .visible_scopes(caller, Permission::ReadProject(ProjectId::new("_")).key())
                    .await?
            }
            wide => wide,
        };
        let page = self
            .project_repo
            .list_by_organization(&query.organization_id, query.pagination.as_ref(), &visible)
            .await?;
        Ok(input.fetched(page))
    }
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

#[async_trait]
impl Run<Fetch<ListProjectMembers>> for ProjectUseCases {
    async fn run(
        &self,
        input: Authorized<ListProjectMembers>,
    ) -> DomainResult<Fetched<ListProjectMembers>> {
        let query = input.command();
        let ids = self
            .project_repo
            .list_principals(&query.project_id, query.pagination.as_ref())
            .await?;
        let users = users_in_order(self.user_repo.as_ref(), ids).await?;
        Ok(input.fetched(users))
    }
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

#[async_trait]
impl Run<Fetch<ListUserProjects>> for ProjectUseCases {
    async fn run(
        &self,
        input: Authorized<ListUserProjects>,
    ) -> DomainResult<Fetched<ListUserProjects>> {
        let query = input.command();
        let page = self
            .project_repo
            .list_for_user(
                &query.user_id,
                Permission::ReadProject(ProjectId::new("_")).key(),
                query.pagination.as_ref(),
            )
            .await?;
        Ok(input.fetched(page))
    }
}
