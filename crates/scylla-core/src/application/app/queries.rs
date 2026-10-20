//! The app's reads. One block per query, in the order it runs: the struct, its access, its
//! output type, what `Fetch` reads.

use super::AppUseCases;
use crate::application::grant::PrincipalAccess;
use crate::domain::app::{App, AppCredential};
use crate::domain::errors::DomainResult;
use crate::domain::ids::{AppId, OrganizationId};
use crate::domain::permission::Permission;
use async_trait::async_trait;
use scylla_extension::{Access, Authorized, Describe, Fetch, Fetched, Query, Run};

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

#[async_trait]
impl Run<Fetch<GetApp>> for AppUseCases {
    async fn run(&self, input: Authorized<GetApp>) -> DomainResult<Fetched<GetApp>> {
        let app = self.app_repo.find_by_id(&input.command().id).await?;
        Ok(input.fetched(app))
    }
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

#[async_trait]
impl Run<Fetch<ListApps>> for AppUseCases {
    async fn run(&self, input: Authorized<ListApps>) -> DomainResult<Fetched<ListApps>> {
        let apps = self
            .app_repo
            .list_by_organization(&input.command().organization_id)
            .await?;
        Ok(input.fetched(apps))
    }
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

#[async_trait]
impl Run<Fetch<ListAppSecrets>> for AppUseCases {
    async fn run(
        &self,
        input: Authorized<ListAppSecrets>,
    ) -> DomainResult<Fetched<ListAppSecrets>> {
        let secrets = self
            .credential_repo
            .list_by_app(&input.command().app_id)
            .await?;
        Ok(input.fetched(secrets))
    }
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

#[async_trait]
impl Run<Fetch<ListAppAccess>> for AppUseCases {
    async fn run(&self, input: Authorized<ListAppAccess>) -> DomainResult<Fetched<ListAppAccess>> {
        let access = self.app_repo.list_access(&input.command().app_id).await?;
        Ok(input.fetched(access))
    }
}
