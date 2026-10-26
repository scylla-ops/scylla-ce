use scylla_core::application::{SecretCipher, SecretRepository};
use scylla_domain::domain::errors::DomainResult;
use scylla_domain::domain::ids::{ProjectId, SecretId};
use scylla_domain::domain::permission::Permission;
use scylla_domain::domain::secret::{Secret, SecretDescription, SecretName, SecretValue};
use scylla_extension::Access;
use std::sync::Arc;

#[strata::module(section = "secrets")]
pub trait Secrets: crate::Access {
    #[component]
    type SecretRepo = Arc<dyn SecretRepository>;

    #[component]
    type Cipher = Arc<dyn SecretCipher>;

    #[usecase(write, access = Access::Requires(Permission::CreateSecret(project_id.clone())))]
    async fn create_secret(
        &self,
        project_id: ProjectId,
        name: SecretName,
        description: SecretDescription,
        value: SecretValue,
    ) -> DomainResult<Secret> {
        let encrypted = self.cipher().encrypt(value.as_str())?;
        let secret = Secret::create(project_id, name, description, encrypted);
        self.secret_repo().create(&secret).await?;
        Ok(secret)
    }

    #[usecase(read, access = Access::Requires(Permission::ListSecrets(project_id.clone())))]
    async fn list_secrets(&self, project_id: ProjectId) -> DomainResult<Vec<Secret>> {
        self.secret_repo().list_by_project(&project_id).await
    }

    #[usecase(write, access = Access::Requires(Permission::DeleteSecret(id.clone())))]
    async fn delete_secret(&self, id: SecretId) -> DomainResult<()> {
        let secret = self.secret_repo().find_by_id(&id).await?;
        self.secret_repo().delete(secret.id()).await
    }
}
