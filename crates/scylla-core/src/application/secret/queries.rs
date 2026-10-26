//! The secret's reads. Each query, its access and its output type. The strata module of the slice
//! runs it.

use crate::domain::ids::ProjectId;
use crate::domain::permission::Permission;
use crate::domain::secret::Secret;
use scylla_extension::{Access, Describe, Query};

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
