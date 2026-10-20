//! The secret's reads. One block per query, in the order it runs: the struct, its access,
//! its output type, what `Fetch` reads.

use super::SecretUseCases;
use crate::domain::errors::DomainResult;
use crate::domain::ids::ProjectId;
use crate::domain::permission::Permission;
use crate::domain::secret::Secret;
use async_trait::async_trait;
use scylla_extension::{Access, Authorized, Describe, Fetch, Fetched, Query, Run};

#[derive(Debug)]
pub struct ListSecrets {
    pub project_id: ProjectId,
}

impl Describe for ListSecrets {
    fn access(&self) -> Access {
        Access::Requires(Permission::ListSecrets(self.project_id.clone()))
    }
}

impl Query for ListSecrets {
    type Output = Vec<Secret>;
}

#[async_trait]
impl Run<Fetch<ListSecrets>> for SecretUseCases {
    async fn run(&self, input: Authorized<ListSecrets>) -> DomainResult<Fetched<ListSecrets>> {
        let secrets = self
            .secret_repo
            .list_by_project(&input.command().project_id)
            .await?;
        Ok(input.fetched(secrets))
    }
}
