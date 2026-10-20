//! The organization's reads. One block per query, in the order it runs: the struct, its
//! access, its output type, what `Fetch` reads.

use super::OrganizationUseCases;
use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::application::user::users_in_order;
use crate::domain::errors::DomainResult;
use crate::domain::ids::{OrganizationId, UserId};
use crate::domain::organization::Organization;
use crate::domain::permission::Permission;
use crate::domain::user::User;
use async_trait::async_trait;
use scylla_extension::{Access, Authorized, Describe, Fetch, Fetched, Query, Run};

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

#[async_trait]
impl Run<Fetch<GetOrganization>> for OrganizationUseCases {
    async fn run(
        &self,
        input: Authorized<GetOrganization>,
    ) -> DomainResult<Fetched<GetOrganization>> {
        let organization = self.org_repo.find_by_id(&input.command().id).await?;
        Ok(input.fetched(organization))
    }
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

#[async_trait]
impl Run<Fetch<ListOrganizations>> for OrganizationUseCases {
    async fn run(
        &self,
        input: Authorized<ListOrganizations>,
    ) -> DomainResult<Fetched<ListOrganizations>> {
        let page = self
            .org_repo
            .list_all(input.command().pagination.as_ref())
            .await?;
        Ok(input.fetched(page))
    }
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

#[async_trait]
impl Run<Fetch<ListOrganizationMembers>> for OrganizationUseCases {
    async fn run(
        &self,
        input: Authorized<ListOrganizationMembers>,
    ) -> DomainResult<Fetched<ListOrganizationMembers>> {
        let query = input.command();
        let ids = self
            .org_repo
            .list_principals(&query.organization_id, query.pagination.as_ref())
            .await?;
        let users = users_in_order(self.user_repo.as_ref(), ids).await?;
        Ok(input.fetched(users))
    }
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

#[async_trait]
impl Run<Fetch<ListUserOrganizations>> for OrganizationUseCases {
    async fn run(
        &self,
        input: Authorized<ListUserOrganizations>,
    ) -> DomainResult<Fetched<ListUserOrganizations>> {
        let query = input.command();
        let page = self
            .org_repo
            .list_for_user(&query.user_id, query.pagination.as_ref())
            .await?;
        Ok(input.fetched(page))
    }
}
