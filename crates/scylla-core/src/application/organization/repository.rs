use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::domain::errors::DomainResult;
use crate::domain::ids::{OrganizationId, UserId};
use crate::domain::organization::Organization;
use async_trait::async_trait;
use scylla_auth::authz::Grant;

#[async_trait]
pub trait OrganizationRepository: Send + Sync {
    async fn create(&self, organization: &Organization) -> DomainResult<Organization>;

    async fn provision_with_owner(
        &self,
        organization: &Organization,
        grant: &Grant,
    ) -> DomainResult<()>;

    /// The members: each user with a grant on the organization or on one of its projects. An app
    /// is not a member.
    async fn list_principals(
        &self,
        org_id: &OrganizationId,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<UserId>>;

    /// The number of users that `list_principals` gives.
    async fn count_members(&self, org_id: &OrganizationId) -> DomainResult<u64>;

    /// `user_id` is in `list_principals`.
    async fn has_member(&self, org_id: &OrganizationId, user_id: &UserId) -> DomainResult<bool>;

    /// System-scoped grants are not expanded: an operator is not a member of every organization.
    async fn list_for_user(
        &self,
        user_id: &UserId,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Organization>>;

    async fn find_by_id(&self, id: &OrganizationId) -> DomainResult<Organization>;

    /// Writes only if the row still carries `organization.version()`, and returns the row with the
    /// bumped version. A stale value is `Stale`; a missing row is `NotFound`.
    async fn update(&self, organization: &Organization) -> DomainResult<Organization>;

    /// Same version rule as `update`.
    async fn delete(&self, organization: &Organization) -> DomainResult<()>;

    async fn list_all(
        &self,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Organization>>;
}
