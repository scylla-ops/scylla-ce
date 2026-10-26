use scylla_core::application::secret::{CreateSecret, DeleteSecret, ListSecrets};
use scylla_core::application::{SecretCipher, SecretRepository};
use scylla_domain::domain::errors::DomainResult;
use scylla_domain::domain::secret::Secret;
use scylla_extension::Describe;
use std::sync::Arc;

#[strata::module(section = "secrets")]
pub trait Secrets: crate::Access {
    #[component]
    type SecretRepo = Arc<dyn SecretRepository>;

    #[component]
    type Cipher = Arc<dyn SecretCipher>;

    #[usecase(write, access = cmd.access())]
    async fn create_secret(&self, cmd: CreateSecret) -> DomainResult<Secret> {
        let encrypted = self.cipher().encrypt(cmd.value.as_str())?;
        let secret = Secret::create(cmd.project_id, cmd.name, cmd.description, encrypted);
        self.secret_repo().create(&secret).await?;
        Ok(secret)
    }

    #[usecase(read, access = query.access())]
    async fn list_secrets(&self, query: ListSecrets) -> DomainResult<Vec<Secret>> {
        self.secret_repo().list_by_project(&query.project_id).await
    }

    #[usecase(write, access = cmd.access())]
    async fn delete_secret(&self, cmd: DeleteSecret) -> DomainResult<()> {
        let secret = self.secret_repo().find_by_id(&cmd.id).await?;
        self.secret_repo().delete(secret.id()).await
    }
}
