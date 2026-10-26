use crate::access::CallerOf;
use crate::ports::JobDispatch;
use scylla_auth::authz::{Grant, ORGANIZATION_ADMIN_ROLE, Principal, Scope};
use scylla_core::application::job::JobScope;
use scylla_core::application::organization::{
    CreateOrganization, DeleteOrganization, GetOrganization, ListOrganizationMembers,
    ListOrganizations, ListUserOrganizations, SetOrganizationActive, UpdateOrganization,
};
use scylla_core::application::pagination::PaginatedResult;
use scylla_core::application::user::users_in_order;
use scylla_core::application::{
    AppRepository, DispatchUseCases, OrganizationRepository, UserRepository,
};
use scylla_domain::domain::caller::CallerContext;
use scylla_domain::domain::errors::DomainResult;
use scylla_domain::domain::organization::Organization;
use scylla_domain::domain::role::RoleName;
use scylla_domain::domain::user::User;
use scylla_extension::Describe;
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

    #[usecase]
    async fn create_organization(&self, cmd: CreateOrganization) -> DomainResult<Organization> {
        self.require_access(cmd.access()).await?;
        let organization = Organization::create(cmd.name, cmd.description)?;
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

    #[usecase]
    async fn get_organization(&self, query: GetOrganization) -> DomainResult<Organization> {
        self.require_access(query.access()).await?;
        self.org_repo().find_by_id(&query.id).await
    }

    #[usecase]
    async fn update_organization(&self, cmd: UpdateOrganization) -> DomainResult<Organization> {
        self.require_access(cmd.access()).await?;
        let mut organization = self.org_repo().find_by_id(&cmd.id).await?;
        if let Some(name) = cmd.name {
            organization.update_name(name)?;
        }
        if let Some(description) = cmd.description {
            organization.update_description(description)?;
        }
        self.org_repo().update(&organization).await
    }

    #[usecase]
    async fn set_organization_active(
        &self,
        cmd: SetOrganizationActive,
    ) -> DomainResult<Organization> {
        self.require_access(cmd.access()).await?;
        let mut organization = self.org_repo().find_by_id(&cmd.id).await?;
        organization.set_active(cmd.is_active);
        self.org_repo().update(&organization).await
    }

    #[usecase]
    async fn delete_organization(&self, cmd: DeleteOrganization) -> DomainResult<()> {
        self.require_access(cmd.access()).await?;
        let organization = self.org_repo().find_by_id(&cmd.id).await?;
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

    #[usecase]
    async fn list_organizations(
        &self,
        query: ListOrganizations,
    ) -> DomainResult<PaginatedResult<Organization>> {
        self.require_access(query.access()).await?;
        self.org_repo().list_all(query.pagination.as_ref()).await
    }

    #[usecase]
    async fn list_organization_members(
        &self,
        query: ListOrganizationMembers,
    ) -> DomainResult<PaginatedResult<User>> {
        self.require_access(query.access()).await?;
        let ids = self
            .org_repo()
            .list_principals(&query.organization_id, query.pagination.as_ref())
            .await?;
        users_in_order(self.org_members().as_ref(), ids).await
    }

    #[usecase]
    async fn list_user_organizations(
        &self,
        query: ListUserOrganizations,
    ) -> DomainResult<PaginatedResult<Organization>> {
        self.require_access(query.access()).await?;
        self.org_repo()
            .list_for_user(&query.user_id, query.pagination.as_ref())
            .await
    }
}
