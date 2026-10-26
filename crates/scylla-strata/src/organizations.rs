use crate::access::CallerOf;
use crate::ports::JobDispatch;
use scylla_auth::authz::{Grant, ORGANIZATION_ADMIN_ROLE, Principal, Scope};
use scylla_core::application::job::JobScope;
use scylla_core::application::pagination::{PaginatedResult, PaginationParams};
use scylla_core::application::user::users_in_order;
use scylla_core::application::{
    AppRepository, DispatchUseCases, OrganizationRepository, UserRepository,
};
use scylla_core::domain::access::Access;
use scylla_domain::domain::caller::CallerContext;
use scylla_domain::domain::errors::DomainResult;
use scylla_domain::domain::ids::{OrganizationId, UserId};
use scylla_domain::domain::organization::{
    Organization, OrganizationDescription, OrganizationName,
};
use scylla_domain::domain::permission::Permission;
use scylla_domain::domain::role::RoleName;
use scylla_domain::domain::user::User;
use std::sync::Arc;
use strata::{BuildCx, BuildError, Component};

pub struct OrgDispatcher(pub Arc<DispatchUseCases>);

impl Component for OrgDispatcher {
    async fn build(cx: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Ok(OrgDispatcher(cx.resource::<JobDispatch>().await?.0))
    }
}

#[strata::module(section = "organizations")]
pub trait Organizations: crate::Access {
    #[component]
    type OrgRepo = Arc<dyn OrganizationRepository>;

    #[component]
    type OrgMembers = Arc<dyn UserRepository>;

    #[component]
    type OrgApps = Arc<dyn AppRepository>;

    #[component]
    type OrgDispatch = OrgDispatcher;

    /// For a user caller, the use case also writes the grant that makes the user the admin of the
    /// organization, in the same transaction.
    #[usecase(write, access = Access::Requires(Permission::CreateOrganization))]
    async fn create_organization(
        &self,
        name: OrganizationName,
        description: Option<OrganizationDescription>,
    ) -> DomainResult<Organization> {
        let organization = Organization::create(name, description)?;
        let owner = match self.ctx().caller() {
            CallerContext::User(user_id) => Some(Grant::new(
                Principal::User(user_id.clone()),
                RoleName::new(ORGANIZATION_ADMIN_ROLE)?,
                Scope::Organization(organization.id().clone()),
            )),
            _ => None,
        };
        match owner {
            Some(grant) => {
                self.org_repo()
                    .provision_with_owner(&organization, &grant)
                    .await?;
            }
            None => {
                self.org_repo().create(&organization).await?;
            }
        }
        Ok(organization)
    }

    #[usecase(read, access = Access::Requires(Permission::ReadOrganization(id.clone())))]
    async fn get_organization(&self, id: OrganizationId) -> DomainResult<Organization> {
        self.org_repo().find_by_id(&id).await
    }

    #[usecase(write, access = Access::Requires(Permission::UpdateOrganization(id.clone())))]
    async fn update_organization(
        &self,
        id: OrganizationId,
        name: Option<OrganizationName>,
        description: Option<Option<OrganizationDescription>>,
    ) -> DomainResult<Organization> {
        let mut organization = self.org_repo().find_by_id(&id).await?;
        if let Some(name) = name {
            organization.update_name(name)?;
        }
        if let Some(description) = description {
            organization.update_description(description)?;
        }
        self.org_repo().update(&organization).await
    }

    #[usecase(write, access = Access::Requires(Permission::UpdateOrganization(id.clone())))]
    async fn set_organization_active(
        &self,
        id: OrganizationId,
        is_active: bool,
    ) -> DomainResult<Organization> {
        let mut organization = self.org_repo().find_by_id(&id).await?;
        organization.set_active(is_active);
        self.org_repo().update(&organization).await
    }

    #[usecase(write, access = Access::Requires(Permission::DeleteOrganization(id.clone())))]
    async fn delete_organization(&self, id: OrganizationId) -> DomainResult<()> {
        let organization = self.org_repo().find_by_id(&id).await?;
        let apps = self
            .org_apps()
            .list_by_organization(organization.id())
            .await?;
        let dispatch = &self.org_dispatch().0;
        dispatch
            .recall(
                JobScope::Organization(organization.id()),
                self.org_repo().delete(&organization),
            )
            .await?;
        for app in &apps {
            dispatch.disconnect(app.id());
        }
        Ok(())
    }

    #[usecase(read, access = Access::Requires(Permission::ListOrganizations))]
    async fn list_organizations(
        &self,
        pagination: Option<PaginationParams>,
    ) -> DomainResult<PaginatedResult<Organization>> {
        self.org_repo().list_all(pagination.as_ref()).await
    }

    #[usecase(read, access = Access::Requires(Permission::ListOrganizationMembers(organization_id.clone())))]
    async fn list_organization_members(
        &self,
        organization_id: OrganizationId,
        pagination: Option<PaginationParams>,
    ) -> DomainResult<PaginatedResult<User>> {
        let ids = self
            .org_repo()
            .list_principals(&organization_id, pagination.as_ref())
            .await?;
        users_in_order(self.org_members().as_ref(), ids).await
    }

    #[usecase(read, access = Access::Requires(Permission::ListUserOrganizations(user_id.clone())))]
    async fn list_user_organizations(
        &self,
        user_id: UserId,
        pagination: Option<PaginationParams>,
    ) -> DomainResult<PaginatedResult<Organization>> {
        self.org_repo()
            .list_for_user(&user_id, pagination.as_ref())
            .await
    }
}
