use crate::application::grant::PrincipalAccess;
use crate::domain::agent::Agent;
use crate::domain::app::{App, AppCredential};
use crate::domain::errors::DomainResult;
use crate::domain::ids::{AppId, OrganizationId};
use async_trait::async_trait;
use scylla_auth::authz::Grant;

#[async_trait]
pub trait AppRepository: Send + Sync {
    async fn create_app(&self, app: &App, credential: &AppCredential) -> DomainResult<()>;
    async fn provision_agent(
        &self,
        app: &App,
        credential: &AppCredential,
        agent: &Agent,
        grant: &Grant,
    ) -> DomainResult<()>;
    /// The App and its grant in one transaction, with no secret: the trigger runner.
    async fn provision(&self, app: &App, grant: &Grant) -> DomainResult<()>;
    async fn find_by_id(&self, id: &AppId) -> DomainResult<App>;
    async fn find_trigger_runner(
        &self,
        organization_id: &OrganizationId,
    ) -> DomainResult<Option<AppId>>;
    async fn list_by_organization(
        &self,
        organization_id: &OrganizationId,
    ) -> DomainResult<Vec<App>>;
    async fn set_active(&self, id: &AppId, active: bool) -> DomainResult<()>;
    async fn delete(&self, id: &AppId) -> DomainResult<()>;
    /// The system grants first, then by organization name, then by project name.
    async fn list_access(&self, id: &AppId) -> DomainResult<Vec<PrincipalAccess>>;
}
