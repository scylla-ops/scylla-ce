//! The projects: the resource that an actor owns in the core.

use time::OffsetDateTime;

use crate::actors::ActorId;
use crate::page::{Page, PageRequest};
use crate::{DomainError, Id, ResourceKind};

/// The kind of a project, for the hooks.
pub const KIND: ResourceKind = ResourceKind::new("project");

/// The id of a project.
pub type ProjectId = Id<Project>;

/// The display name of a project: trimmed, 1 to `MAX_LEN` characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectName(String);

impl ProjectName {
    /// The maximum number of characters, after trim.
    pub const MAX_LEN: usize = 100;

    /// Trims the text and checks its length.
    pub fn parse(raw: String) -> Result<Self, DomainError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(DomainError::Empty);
        }
        if trimmed.chars().count() > Self::MAX_LEN {
            return Err(DomainError::TooLong { max: Self::MAX_LEN });
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// Returns the name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A project, as the store returns it.
#[derive(Clone, Debug)]
pub struct Project {
    /// The id of the project.
    pub id: ProjectId,
    /// The actor that owns the project.
    pub owner_id: ActorId,
    /// The actor that created the project.
    pub created_by: ActorId,
    /// The display name.
    pub name: String,
    /// The creation time, set by the database.
    pub created_at: OffsetDateTime,
}

/// The input of a project creation.
#[derive(Clone, Debug)]
pub struct NewProject {
    /// The actor that owns the new project.
    pub owner_id: ActorId,
    /// The actor that creates the project.
    pub created_by: ActorId,
    /// The display name.
    pub name: ProjectName,
}

/// The input of a project list.
#[derive(Clone, Copy, Debug)]
pub struct ListProjects {
    /// The owner of the projects.
    pub owner_id: ActorId,
    /// The page to return.
    pub page: PageRequest<ProjectId>,
}

/// One page of projects.
pub type ProjectPage = Page<Project, ProjectId>;
