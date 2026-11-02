//! The projects: the resource that an actor owns in the core.

use time::OffsetDateTime;

use crate::actors::{Actor, ActorId};
use crate::authz::Resource;
use crate::page::{Page, PageRequest};
use crate::{DomainError, Id, ResourceKind};

/// The kind of a project, for the hooks: the name of its definition.
pub const KIND: ResourceKind = ResourceKind::new(Project::NAME);

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

crate::authz! {
    /// The grants on one project. The owner is in Postgres, not here. `delete`, `edit` and
    /// `view` are groups, and each route has its own permission.
    definition project for Project {
        /// An actor that can do everything on this project.
        relation admin: Actor;
        /// An actor that can change this project.
        relation editor: Actor;
        /// An actor that can read this project.
        relation viewer: Actor;
        /// The group of the routes that delete the project: its admins and their delegates.
        permission delete = admin + admin->act_for;
        /// The group of the routes that change the project.
        permission edit = delete + editor + editor->act_for;
        /// The group of the routes that read the project.
        permission view = edit + viewer + viewer->act_for;
        /// The route `DeleteProject`, on one project.
        permission delete_project = delete;
    }

    /// The routes of the projects on their owner. A grant here gives the route on each project
    /// of the owner.
    extend Actor as ProjectOwner {
        /// The route `CreateProject`, for this owner.
        permission create_project = act_for;
        /// The route `ListProjects`, for this owner.
        permission list_projects = act_for;
        /// The route `DeleteProject`, on each project of this owner.
        permission delete_project = act_for;
    }
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
