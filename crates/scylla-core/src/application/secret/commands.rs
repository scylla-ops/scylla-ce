//! The secret's writes. Each command, its access and its payload types. The strata module of the
//! slice runs it.

use crate::domain::ids::{ProjectId, SecretId};
use crate::domain::permission::Permission;
use crate::domain::secret::{Secret, SecretDescription, SecretName, SecretValue};
use scylla_extension::{Access, Command, Deleted, Describe, Draft};

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
