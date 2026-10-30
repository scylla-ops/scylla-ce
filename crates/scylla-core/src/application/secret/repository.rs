use crate::domain::errors::DomainResult;
use crate::domain::ids::{ProjectId, SecretId};
use crate::domain::secret::{Secret, SecretChange};
use async_trait::async_trait;

#[async_trait]
pub trait SecretRepository: Send + Sync {
    async fn create(&self, secret: &Secret) -> DomainResult<()>;
    async fn find_by_id(&self, id: &SecretId) -> DomainResult<Secret>;
    async fn list_by_project(&self, project_id: &ProjectId) -> DomainResult<Vec<Secret>>;
    async fn update(&self, change: &SecretChange) -> DomainResult<Secret>;
    async fn delete(&self, id: &SecretId) -> DomainResult<()>;
}
