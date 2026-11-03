//! The projects: the unit of work in a workspace. Their secrets are in `secrets`.

pub mod secrets;

use time::OffsetDateTime;

use crate::authz::Resource;
use crate::identity::actors::{Actor, ActorId};
use crate::tenancy::workspaces::{Workspace, WorkspaceId};
use crate::{DomainError, Name};
use crate::{Id, ResourceKind};
use crate::{Page, PageRequest};

/// The kind of a project, for the hooks: the name of its definition.
pub const KIND: ResourceKind = ResourceKind::new(Project::NAME);

/// The id of a project.
pub type ProjectId = Id<Project>;

/// The display name of a project.
pub type ProjectName = Name;

/// A project, as the store returns it.
#[derive(Clone, Debug)]
pub struct Project {
    /// The id of the project.
    pub id: ProjectId,
    /// The workspace that holds the project.
    pub workspace_id: WorkspaceId,
    /// The actor that created the project.
    pub created_by: ActorId,
    /// The display name.
    pub name: String,
    /// The description. Empty if the project has none.
    pub description: String,
    /// The creation time, set by the database.
    pub created_at: OffsetDateTime,
}

/// The description of a project: trimmed, at most `MAX_LEN` characters, on one line or more.
/// An empty description is no description.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectDescription(String);

impl ProjectDescription {
    /// The maximum number of characters, after trim.
    pub const MAX_LEN: usize = 1024;

    /// Trims the text, and checks its length and that its only control characters are line
    /// breaks and tabs.
    pub fn parse(raw: String) -> Result<Self, DomainError> {
        let trimmed = raw.trim();
        if trimmed.chars().count() > Self::MAX_LEN {
            return Err(DomainError::TooLong { max: Self::MAX_LEN });
        }
        let allowed = |c: char| !c.is_control() || matches!(c, '\n' | '\r' | '\t');
        if !trimmed.chars().all(allowed) {
            return Err(DomainError::ControlCharacter);
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// Returns the description as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

crate::authz! {
    /// The roles on one project. Its workspace is in Postgres, not here. `delete`, `edit` and
    /// `view` are groups, and each route has its own permission.
    definition project for Project {
        /// An actor that can do everything on this project.
        relation admin: Actor;
        /// An actor that can change this project.
        relation editor: Actor;
        /// An actor that can read this project.
        relation viewer: Actor;
        /// The group of the routes that only the admins use: delete the project, and change
        /// its secrets.
        permission delete = admin;
        /// The group of the routes that change the project.
        permission edit = delete + editor;
        /// The group of the routes that read the project.
        permission view = edit + viewer;
        /// The route `GetProject`, on one project.
        permission get_project = view;
        /// The route `ListProjects`: a project that the caller can view is in its list.
        permission list_projects = view;
        /// The route `UpdateProject`, on one project.
        permission update_project = edit;
        /// The route `DeleteProject`, on one project.
        permission delete_project = delete;
        /// The route `CreateSecret` of `SecretService`, in this project.
        permission create_secret = delete;
        /// The route `UpdateSecret` of `SecretService`, on each secret of this project.
        permission update_secret = delete;
        /// The route `DeleteSecret` of `SecretService`, on each secret of this project.
        permission delete_secret = delete;
        /// The route `ListSecrets` of `SecretService`: the names of the secrets, never a value.
        permission list_secrets = edit;
        /// The route `Grant` of `AccessService`: give a role on this project. Only its admins.
        permission grant = admin;
        /// The route `Revoke` of `AccessService`: take back a role on this project.
        permission revoke = admin;
        /// The route `ListAccess` of `AccessService`: the roles on this project.
        permission list_access = view;
    }

    /// The routes of the projects and of their secrets on their workspace. A role there gives
    /// the route on each project of the workspace.
    extend Workspace as ProjectHolder {
        /// The route `CreateProject`, in this workspace.
        permission create_project = work;
        /// The route `ListProjects`: the projects of this workspace are in the list.
        permission list_projects = read;
        /// The route `GetProject`, on each project of this workspace.
        permission get_project = read;
        /// The route `UpdateProject`, on each project of this workspace.
        permission update_project = work;
        /// The route `DeleteProject`, on each project of this workspace.
        permission delete_project = manage;
        /// The route `CreateSecret` of `SecretService`, in each project of this workspace.
        permission create_secret = manage;
        /// The route `UpdateSecret` of `SecretService`, in each project of this workspace.
        permission update_secret = manage;
        /// The route `DeleteSecret` of `SecretService`, in each project of this workspace.
        permission delete_secret = manage;
        /// The route `ListSecrets` of `SecretService`, in each project of this workspace.
        permission list_secrets = work;
    }

    /// The routes of the projects on the caller itself.
    extend Actor as ProjectCaller {
        /// The route `ListProjects`: each caller lists the projects that it can see.
        permission list_projects = nil;
    }
}

/// The input of a project creation.
#[derive(Clone, Debug)]
pub struct NewProject {
    /// The workspace of the new project.
    pub workspace_id: WorkspaceId,
    /// The actor that creates the project.
    pub created_by: ActorId,
    /// The display name.
    pub name: ProjectName,
    /// The description, empty for none.
    pub description: ProjectDescription,
}

/// A change of a project. A field that is `None` does not change.
#[derive(Clone, Debug, Default)]
pub struct ProjectChange {
    /// The new display name.
    pub name: Option<ProjectName>,
    /// The new description. An empty one removes the description.
    pub description: Option<ProjectDescription>,
}

/// The input of a project list: the projects that the caller can see.
#[derive(Clone, Copy, Debug)]
pub struct ListProjects {
    /// Only the projects of this workspace, if it is set.
    pub workspace_id: Option<WorkspaceId>,
    /// The page to return.
    pub page: PageRequest<ProjectId>,
}

/// What a caller can see: all the projects of some workspaces, and some other projects.
#[derive(Clone, Debug)]
pub struct Visible {
    /// The caller: the projects of the workspaces that it owns are visible.
    pub caller: ActorId,
    /// The workspaces where the caller has a role that lists their projects.
    pub workspaces: Vec<WorkspaceId>,
    /// The projects shared with the caller.
    pub projects: Vec<ProjectId>,
}

/// One page of projects.
pub type ProjectPage = Page<Project, ProjectId>;
