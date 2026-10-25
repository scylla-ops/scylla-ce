//! The secret's writes. One block per command, in the order it runs: the struct, its
//! access, its payload types, what `Prepare` builds, what `Persist` writes.

use super::SecretUseCases;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{ProjectId, SecretId};
use crate::domain::permission::Permission;
use crate::domain::secret::{Secret, SecretChange, SecretDescription, SecretName, SecretValue};
use async_trait::async_trait;
use scylla_extension::{
    Access, Authorized, Command, Committed, Deleted, Describe, Draft, Persist, Prepare, Prepared,
    Run,
};

#[derive(Debug)]
pub struct CreateSecret {
    pub project_id: ProjectId,
    pub name: SecretName,
    pub description: SecretDescription,
    pub value: SecretValue,
}

impl Describe for CreateSecret {
    fn access(&self) -> Access {
        Access::Requires(Permission::CreateSecret(self.project_id.clone()))
    }
}

impl Command for CreateSecret {
    type Staged = Draft<Secret>;
    type Committed = Secret;
}

#[async_trait]
impl Run<Prepare<CreateSecret>> for SecretUseCases {
    async fn run(&self, input: Authorized<CreateSecret>) -> DomainResult<Prepared<CreateSecret>> {
        let cmd = input.command();
        let encrypted = self.cipher.encrypt(cmd.value.as_str())?;
        let secret = Secret::create(
            cmd.project_id.clone(),
            cmd.name.clone(),
            cmd.description.clone(),
            encrypted,
        );
        Ok(input.prepared(Draft::new(secret)))
    }
}

#[async_trait]
impl Run<Persist<CreateSecret>> for SecretUseCases {
    async fn run(&self, input: Prepared<CreateSecret>) -> DomainResult<Committed<CreateSecret>> {
        input
            .commit(async |draft| {
                let secret = draft.into_inner();
                self.secret_repo.create(&secret).await?;
                Ok(secret)
            })
            .await
    }
}

#[derive(Debug)]
pub struct DeleteSecret {
    pub id: SecretId,
}

impl Describe for DeleteSecret {
    fn access(&self) -> Access {
        Access::Requires(Permission::DeleteSecret(self.id.clone()))
    }
}

impl Command for DeleteSecret {
    type Staged = Secret;
    type Committed = Deleted<Secret>;
}

#[async_trait]
impl Run<Prepare<DeleteSecret>> for SecretUseCases {
    async fn run(&self, input: Authorized<DeleteSecret>) -> DomainResult<Prepared<DeleteSecret>> {
        let secret = self.secret_repo.find_by_id(&input.command().id).await?;
        Ok(input.prepared(secret))
    }
}

#[async_trait]
impl Run<Persist<DeleteSecret>> for SecretUseCases {
    async fn run(&self, input: Prepared<DeleteSecret>) -> DomainResult<Committed<DeleteSecret>> {
        input
            .commit(async |secret| {
                self.secret_repo.delete(secret.id()).await?;
                Ok(Deleted::new(secret))
            })
            .await
    }
}

#[derive(Debug)]
pub struct UpdateSecret {
    pub id: SecretId,
    pub value: Option<SecretValue>,
    pub description: Option<SecretDescription>,
}

impl Describe for UpdateSecret {
    fn access(&self) -> Access {
        Access::Requires(Permission::UpdateSecret(self.id.clone()))
    }
}

impl Command for UpdateSecret {
    type Staged = Draft<SecretChange>;
    type Committed = Secret;
}

#[async_trait]
impl Run<Prepare<UpdateSecret>> for SecretUseCases {
    async fn run(&self, input: Authorized<UpdateSecret>) -> DomainResult<Prepared<UpdateSecret>> {
        let cmd = input.command();
        if cmd.value.is_none() && cmd.description.is_none() {
            return Err(DomainError::validation(
                "An update needs a value or a description",
            ));
        }
        let secret = self.secret_repo.find_by_id(&cmd.id).await?;
        let encrypted = cmd
            .value
            .as_ref()
            .map(|value| self.cipher.encrypt(value.as_str()))
            .transpose()?;
        let change = SecretChange::new(secret.id().clone(), cmd.description.clone(), encrypted);
        Ok(input.prepared(Draft::new(change)))
    }
}

#[async_trait]
impl Run<Persist<UpdateSecret>> for SecretUseCases {
    async fn run(&self, input: Prepared<UpdateSecret>) -> DomainResult<Committed<UpdateSecret>> {
        input
            .commit(async |draft| self.secret_repo.update(&draft.into_inner()).await)
            .await
    }
}
