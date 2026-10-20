mod description;
mod name;

pub use description::*;
pub use name::*;

use crate::domain::clock;
use crate::domain::errors::DomainResult;
use crate::domain::ids::{OrganizationId, ProjectId};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct Project {
    id: ProjectId,
    name: ProjectName,
    description: Option<ProjectDescription>,
    organization_id: OrganizationId,
    is_active: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    /// The row version the value was read at. The store checks it on every write and bumps
    /// it itself; the domain never changes it.
    version: u64,
}

impl Project {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn from_persistence(
        id: ProjectId,
        name: ProjectName,
        description: Option<ProjectDescription>,
        organization_id: OrganizationId,
        is_active: bool,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        version: u64,
    ) -> Self {
        Self {
            id,
            name,
            description,
            organization_id,
            is_active,
            created_at,
            updated_at,
            version,
        }
    }

    pub fn create(
        name: ProjectName,
        description: Option<ProjectDescription>,
        organization_id: OrganizationId,
    ) -> DomainResult<Self> {
        let now = clock::now();
        Ok(Self {
            id: ProjectId::generate(),
            name,
            description,
            organization_id,
            is_active: true,
            created_at: now,
            updated_at: now,
            version: 0,
        })
    }

    pub fn update_name(&mut self, name: ProjectName) -> DomainResult<()> {
        self.name = name;
        self.updated_at = clock::now();
        Ok(())
    }

    pub fn update_description(
        &mut self,
        description: Option<ProjectDescription>,
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
    pub fn id(&self) -> &ProjectId {
        &self.id
    }

    #[must_use]
    pub fn name(&self) -> &ProjectName {
        &self.name
    }

    #[must_use]
    pub fn description(&self) -> Option<&ProjectDescription> {
        self.description.as_ref()
    }

    #[must_use]
    pub fn organization_id(&self) -> &OrganizationId {
        &self.organization_id
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
    pub fn version(&self) -> u64 {
        self.version
    }

    #[must_use]
    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}
