mod name;
mod value;

pub use name::*;
pub use value::*;

use crate::domain::clock;
use crate::domain::ids::{ProjectId, SecretId};
use crate::domain::text::Description;
use chrono::{DateTime, Utc};

pub type SecretDescription = Description;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretChange {
    id: SecretId,
    description: Option<SecretDescription>,
    encrypted_value: Option<Vec<u8>>,
    updated_at: DateTime<Utc>,
}

impl SecretChange {
    #[must_use]
    pub fn new(
        id: SecretId,
        description: Option<SecretDescription>,
        encrypted_value: Option<Vec<u8>>,
    ) -> Self {
        Self {
            id,
            description,
            encrypted_value,
            updated_at: clock::now(),
        }
    }

    #[must_use]
    pub fn id(&self) -> &SecretId {
        &self.id
    }

    #[must_use]
    pub fn description(&self) -> Option<&SecretDescription> {
        self.description.as_ref()
    }

    #[must_use]
    pub fn encrypted_value(&self) -> Option<&[u8]> {
        self.encrypted_value.as_deref()
    }

    #[must_use]
    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Secret {
    id: SecretId,
    project_id: ProjectId,
    name: SecretName,
    description: SecretDescription,
    encrypted_value: Vec<u8>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl Secret {
    #[must_use]
    pub fn create(
        project_id: ProjectId,
        name: SecretName,
        description: SecretDescription,
        encrypted_value: Vec<u8>,
    ) -> Self {
        let now = clock::now();
        Self {
            id: SecretId::generate(),
            project_id,
            name,
            description,
            encrypted_value,
            created_at: now,
            updated_at: now,
        }
    }

    #[must_use]
    pub fn from_persistence(
        id: SecretId,
        project_id: ProjectId,
        name: SecretName,
        description: SecretDescription,
        encrypted_value: Vec<u8>,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            project_id,
            name,
            description,
            encrypted_value,
            created_at,
            updated_at,
        }
    }

    #[must_use]
    pub fn id(&self) -> &SecretId {
        &self.id
    }

    #[must_use]
    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    #[must_use]
    pub fn name(&self) -> &SecretName {
        &self.name
    }

    #[must_use]
    pub fn description(&self) -> &SecretDescription {
        &self.description
    }

    #[must_use]
    pub fn encrypted_value(&self) -> &[u8] {
        &self.encrypted_value
    }

    #[must_use]
    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    #[must_use]
    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}
