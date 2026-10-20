//! The organization's writes. One block per command, in the order it runs: the struct, its
//! access, its payload types, what `Prepare` builds, what `Persist` writes.

use super::OrganizationUseCases;
use crate::application::job::JobScope;
use crate::domain::caller::CallerContext;
use crate::domain::errors::DomainResult;
use crate::domain::ids::OrganizationId;
use crate::domain::organization::{Organization, OrganizationDescription, OrganizationName};
use crate::domain::permission::Permission;
use crate::domain::role::RoleName;
use async_trait::async_trait;
use scylla_auth::authz::{Grant, ORGANIZATION_ADMIN_ROLE, Principal, Scope};
use scylla_extension::{
    Access, Authorized, Command, Committed, Deleted, Describe, Draft, Persist, Prepare, Prepared,
    Run,
};

#[derive(Debug)]
pub struct CreateOrganization {
    pub name: OrganizationName,
    pub description: Option<OrganizationDescription>,
}

/// For a user caller, the grant that makes them the organization's admin; both rows go in one
/// transaction, so they are staged together.
#[derive(Debug)]
pub struct NewOrganization {
    pub organization: Organization,
    pub owner: Option<Grant>,
}

impl Describe for CreateOrganization {
    fn access(&self) -> Access {
        Access::Requires(Permission::CreateOrganization)
    }
}

impl Command for CreateOrganization {
    type Staged = Draft<NewOrganization>;
    type Committed = Organization;
}

#[async_trait]
impl Run<Prepare<CreateOrganization>> for OrganizationUseCases {
    async fn run(
        &self,
        input: Authorized<CreateOrganization>,
    ) -> DomainResult<Prepared<CreateOrganization>> {
        let cmd = input.command();
        let organization = Organization::create(cmd.name.clone(), cmd.description.clone())?;
        let owner = match input.caller() {
            CallerContext::User(user_id) => Some(Grant::new(
                Principal::User(user_id.clone()),
                RoleName::new(ORGANIZATION_ADMIN_ROLE)?,
                Scope::Organization(organization.id().clone()),
            )),
            _ => None,
        };
        Ok(input.prepared(Draft::new(NewOrganization {
            organization,
            owner,
        })))
    }
}

#[async_trait]
impl Run<Persist<CreateOrganization>> for OrganizationUseCases {
    async fn run(
        &self,
        input: Prepared<CreateOrganization>,
    ) -> DomainResult<Committed<CreateOrganization>> {
        input
            .commit(async |draft| {
                let NewOrganization {
                    organization,
                    owner,
                } = draft.into_inner();
                match owner {
                    Some(grant) => {
                        self.org_repo
                            .provision_with_owner(&organization, &grant)
                            .await?;
                    }
                    None => {
                        self.org_repo.create(&organization).await?;
                    }
                }
                Ok(organization)
            })
            .await
    }
}

#[derive(Debug)]
pub struct UpdateOrganization {
    pub id: OrganizationId,
    pub name: Option<OrganizationName>,
    pub description: Option<Option<OrganizationDescription>>,
}

impl Describe for UpdateOrganization {
    fn access(&self) -> Access {
        Access::Requires(Permission::UpdateOrganization(self.id.clone()))
    }
}

impl Command for UpdateOrganization {
    type Staged = Draft<Organization>;
    type Committed = Organization;
}

#[async_trait]
impl Run<Prepare<UpdateOrganization>> for OrganizationUseCases {
    async fn run(
        &self,
        input: Authorized<UpdateOrganization>,
    ) -> DomainResult<Prepared<UpdateOrganization>> {
        let cmd = input.command();
        let mut organization = self.org_repo.find_by_id(&cmd.id).await?;
        if let Some(name) = &cmd.name {
            organization.update_name(name.clone())?;
        }
        if let Some(description) = &cmd.description {
            organization.update_description(description.clone())?;
        }
        Ok(input.prepared(Draft::new(organization)))
    }
}

#[async_trait]
impl Run<Persist<UpdateOrganization>> for OrganizationUseCases {
    async fn run(
        &self,
        input: Prepared<UpdateOrganization>,
    ) -> DomainResult<Committed<UpdateOrganization>> {
        input
            .commit(async |draft| self.org_repo.update(&draft.into_inner()).await)
            .await
    }
}

#[derive(Debug)]
pub struct SetOrganizationActive {
    pub id: OrganizationId,
    pub is_active: bool,
}

impl Describe for SetOrganizationActive {
    fn access(&self) -> Access {
        Access::Requires(Permission::UpdateOrganization(self.id.clone()))
    }
}

impl Command for SetOrganizationActive {
    type Staged = Draft<Organization>;
    type Committed = Organization;
}

#[async_trait]
impl Run<Prepare<SetOrganizationActive>> for OrganizationUseCases {
    async fn run(
        &self,
        input: Authorized<SetOrganizationActive>,
    ) -> DomainResult<Prepared<SetOrganizationActive>> {
        let cmd = input.command();
        let mut organization = self.org_repo.find_by_id(&cmd.id).await?;
        organization.set_active(cmd.is_active);
        Ok(input.prepared(Draft::new(organization)))
    }
}

#[async_trait]
impl Run<Persist<SetOrganizationActive>> for OrganizationUseCases {
    async fn run(
        &self,
        input: Prepared<SetOrganizationActive>,
    ) -> DomainResult<Committed<SetOrganizationActive>> {
        input
            .commit(async |draft| self.org_repo.update(&draft.into_inner()).await)
            .await
    }
}

#[derive(Debug)]
pub struct DeleteOrganization {
    pub id: OrganizationId,
}

impl Describe for DeleteOrganization {
    fn access(&self) -> Access {
        Access::Requires(Permission::DeleteOrganization(self.id.clone()))
    }
}

impl Command for DeleteOrganization {
    type Staged = Organization;
    type Committed = Deleted<Organization>;
}

#[async_trait]
impl Run<Prepare<DeleteOrganization>> for OrganizationUseCases {
    async fn run(
        &self,
        input: Authorized<DeleteOrganization>,
    ) -> DomainResult<Prepared<DeleteOrganization>> {
        let organization = self.org_repo.find_by_id(&input.command().id).await?;
        Ok(input.prepared(organization))
    }
}

#[async_trait]
impl Run<Persist<DeleteOrganization>> for OrganizationUseCases {
    async fn run(
        &self,
        input: Prepared<DeleteOrganization>,
    ) -> DomainResult<Committed<DeleteOrganization>> {
        input
            .commit(async |organization| {
                let apps = self
                    .app_repo
                    .list_by_organization(organization.id())
                    .await?;
                self.dispatch
                    .recall(
                        JobScope::Organization(organization.id()),
                        self.org_repo.delete(&organization),
                    )
                    .await?;
                for app in &apps {
                    self.dispatch.disconnect(app.id());
                }
                Ok(Deleted::new(organization))
            })
            .await
    }
}
