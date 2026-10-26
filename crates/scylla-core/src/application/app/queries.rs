//! The app's reads. Each query, its access and its output type. The strata module of the slice runs
//! it.

use crate::application::grant::PrincipalAccess;
use crate::domain::app::{App, AppCredential};
use crate::domain::ids::{AppId, OrganizationId};
use crate::domain::permission::Permission;
use scylla_extension::{Access, Describe, Query};

#[derive(Debug)]
pub struct GetApp {
    pub id: AppId,
}

impl Describe for GetApp {
    fn access(&self) -> Access {
        Access::Requires(Permission::ReadApp(self.id.clone()))
    }
}

impl Query for GetApp {
    type Output = App;
}

#[derive(Debug)]
pub struct ListApps {
    pub organization_id: OrganizationId,
}

impl Describe for ListApps {
    fn access(&self) -> Access {
        Access::Requires(Permission::ListAppsByOrganization(
            self.organization_id.clone(),
        ))
    }
}

impl Query for ListApps {
    type Output = Vec<App>;
}

#[derive(Debug)]
pub struct ListAppSecrets {
    pub app_id: AppId,
}

impl Describe for ListAppSecrets {
    fn access(&self) -> Access {
        Access::Requires(Permission::ReadApp(self.app_id.clone()))
    }
}

impl Query for ListAppSecrets {
    type Output = Vec<AppCredential>;
}

/// An unknown App gives an empty list: the store reads only the grants.
#[derive(Debug)]
pub struct ListAppAccess {
    pub app_id: AppId,
}

impl Describe for ListAppAccess {
    fn access(&self) -> Access {
        Access::Requires(Permission::ReadApp(self.app_id.clone()))
    }
}

impl Query for ListAppAccess {
    type Output = Vec<PrincipalAccess>;
}
