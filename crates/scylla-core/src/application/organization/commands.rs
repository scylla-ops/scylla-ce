//! The organization's writes. Each command, its access and its payload types. The strata module of
//! the slice runs it.

use crate::domain::ids::OrganizationId;
use crate::domain::organization::{Organization, OrganizationDescription, OrganizationName};
use crate::domain::permission::Permission;
use scylla_auth::authz::Grant;
use scylla_extension::{Access, Command, Deleted, Describe, Draft};

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
