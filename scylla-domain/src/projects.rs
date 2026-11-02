//! The projects: the unit of work in a workspace.

use time::OffsetDateTime;

use crate::actors::{Actor, ActorId};
use crate::authz::Resource;
use crate::name::Name;
use crate::page::{Page, PageRequest};
use crate::workspaces::{Workspace, WorkspaceId};
use crate::{Id, ResourceKind};

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
    /// The creation time, set by the database.
    pub created_at: OffsetDateTime,
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
        /// The group of the routes that delete the project.
        permission delete = admin;
        /// The group of the routes that change the project.
        permission edit = delete + editor;
        /// The group of the routes that read the project.
        permission view = edit + viewer;
        /// The route `GetProject`, on one project.
        permission get_project = view;
        /// The route `ListProjects`: a project that the caller can view is in its list.
        permission list_projects = view;
        /// The route `DeleteProject`, on one project.
        permission delete_project = delete;
        /// The route `Grant` of `AccessService`: give a role on this project. Only its admins.
        permission grant = admin;
        /// The route `Revoke` of `AccessService`: take back a role on this project.
        permission revoke = admin;
    }

    /// The routes of the projects on their workspace. A role there gives the route on each
    /// project of the workspace.
    extend Workspace as ProjectHolder {
        /// The route `CreateProject`, in this workspace.
        permission create_project = work;
        /// The route `ListProjects`: the projects of this workspace are in the list.
        permission list_projects = read;
        /// The route `GetProject`, on each project of this workspace.
        permission get_project = read;
        /// The route `DeleteProject`, on each project of this workspace.
        permission delete_project = manage;
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
