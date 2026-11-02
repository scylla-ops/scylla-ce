use time::OffsetDateTime;
use uuid::Uuid;

use crate::users::UserId;
use crate::{DomainError, ResourceKind};

pub const KIND: ResourceKind = ResourceKind::new("project");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProjectId(pub Uuid);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectName(String);

impl ProjectName {
    pub const MAX_LEN: usize = 100;

    pub fn parse(raw: String) -> Result<Self, DomainError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(DomainError::EmptyProjectName);
        }
        if trimmed.chars().count() > Self::MAX_LEN {
            return Err(DomainError::ProjectNameTooLong { max: Self::MAX_LEN });
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct Project {
    pub id: ProjectId,
    pub owner_id: UserId,
    pub name: String,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug)]
pub struct NewProject {
    pub owner_id: UserId,
    pub name: ProjectName,
}

#[derive(Clone, Copy, Debug)]
pub struct ListProjects {
    pub owner_id: UserId,
    pub after: Option<ProjectId>,
    pub limit: usize,
}

#[derive(Clone, Debug)]
pub struct ProjectPage {
    pub projects: Vec<Project>,
    pub next_after: Option<ProjectId>,
}
