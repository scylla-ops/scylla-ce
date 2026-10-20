mod description;
mod name;

pub use description::*;
pub use name::*;

use crate::domain::clock;
use crate::domain::errors::DomainResult;
use crate::domain::ids::OrganizationId;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct Organization {
    id: OrganizationId,
    name: OrganizationName,
    description: Option<OrganizationDescription>,
    is_active: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    /// The row version the value was read at. The store checks it on every write and bumps
    /// it itself; the domain never changes it.
    version: u64,
}

impl Organization {
    #[must_use]
    pub fn from_persistence(
        id: OrganizationId,
        name: OrganizationName,
        description: Option<OrganizationDescription>,
        is_active: bool,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        version: u64,
    ) -> Self {
        Self {
            id,
            name,
            description,
            is_active,
            created_at,
            updated_at,
            version,
        }
    }

    pub fn create(
        name: OrganizationName,
        description: Option<OrganizationDescription>,
    ) -> DomainResult<Self> {
        let now = clock::now();
        Ok(Self {
            id: OrganizationId::generate(),
            name,
            description,
            is_active: true,
            created_at: now,
            updated_at: now,
            version: 0,
        })
    }

    pub fn update_name(&mut self, name: OrganizationName) -> DomainResult<()> {
        self.name = name;
        self.updated_at = clock::now();
        Ok(())
    }

    pub fn update_description(
        &mut self,
        description: Option<OrganizationDescription>,
    ) -> DomainResult<()> {
        self.description = description;
        self.updated_at = clock::now();
        Ok(())
    }

    pub fn set_active(&mut self, is_active: bool) {
        if self.is_active == is_active {
            return;
        }
        self.is_active = is_active;
        self.updated_at = clock::now();
    }

    #[must_use]
    pub fn id(&self) -> &OrganizationId {
        &self.id
    }

    #[must_use]
    pub fn name(&self) -> &OrganizationName {
        &self.name
    }

    #[must_use]
    pub fn description(&self) -> Option<&OrganizationDescription> {
        self.description.as_ref()
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.is_active
    }

    #[must_use]
    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    #[must_use]
    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }

    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }
}
