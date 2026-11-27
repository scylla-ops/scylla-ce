//! The groups of the projects of a workspace, and the order of the projects. The members of the
//! workspace share them: the order of the projects page.

use time::OffsetDateTime;

use crate::identity::actors::ActorId;
use crate::platform::images::IconColor;
use crate::projects::{Project, ProjectId};
use crate::tenancy::workspaces::{Workspace, WorkspaceId};
use crate::{Id, Name, PageRequest, ResourceKind};

/// The kind of a group of projects, for the hooks.
pub const KIND: ResourceKind = ResourceKind::new("project_group");

/// The id of a group of projects.
pub type ProjectGroupId = Id<ProjectGroup>;

/// The display name of a group of projects.
pub type ProjectGroupName = Name;

/// A group of projects, as the store returns it.
#[derive(Clone, Debug)]
pub struct ProjectGroup {
    /// The id of the group.
    pub id: ProjectGroupId,
    /// The workspace of the group.
    pub workspace_id: WorkspaceId,
    /// The actor that created the group.
    pub created_by: ActorId,
    /// The display name.
    pub name: String,
    /// The creation time, set by the database.
    pub created_at: OffsetDateTime,
    /// The color of the folder of the group, or `None` for the default color.
    pub color: Option<IconColor>,
}

/// The input of a group creation.
#[derive(Clone, Debug)]
pub struct NewProjectGroup {
    /// The workspace of the new group.
    pub workspace_id: WorkspaceId,
    /// The actor that creates the group.
    pub created_by: ActorId,
    /// The display name.
    pub name: ProjectGroupName,
    /// The color of the folder, or `None` for the default color.
    pub color: Option<IconColor>,
}

/// A change of a group. A field that is `None` does not change.
#[derive(Clone, Debug, Default)]
pub struct ProjectGroupChange {
    /// The new display name.
    pub name: Option<ProjectGroupName>,
    /// The new place of the group in the order of the groups, from 0.
    pub position: Option<usize>,
    /// The new color of the folder. `Some(None)` gives the default color again.
    pub color: Option<Option<IconColor>>,
}

/// A move of a project to a group and to a place there.
#[derive(Clone, Copy, Debug)]
pub struct ProjectMove {
    /// The project to move.
    pub project_id: ProjectId,
    /// The group of the project after the move. `None`: no group.
    pub group_id: Option<ProjectGroupId>,
    /// The project that comes just after the moved project. `None`: the moved project goes last.
    pub before: Option<ProjectId>,
}

/// The input of a list of the groups of a workspace.
#[derive(Clone, Copy, Debug)]
pub struct ListProjectGroups {
    /// The workspace.
    pub workspace_id: WorkspaceId,
    /// The page to return.
    pub page: PageRequest<ProjectGroupId>,
}

/// Puts an item at a place in an order: before another item, or last when the other item is not
/// in the order. The item goes out of its old place first.
pub fn place<T: PartialEq + Copy>(order: &mut Vec<T>, item: T, before: Option<T>) {
    order.retain(|other| *other != item);
    let index = before
        .and_then(|before| order.iter().position(|other| *other == before))
        .unwrap_or(order.len());
    order.insert(index, item);
}

/// Puts an item at an index in an order, or last when the index is after the last item. The item
/// goes out of its old place first.
pub fn place_at<T: PartialEq + Copy>(order: &mut Vec<T>, item: T, index: usize) {
    order.retain(|other| *other != item);
    let index = index.min(order.len());
    order.insert(index, item);
}

crate::authz! {
    /// The routes of the groups of the projects and of their order, on their workspace. A role
    /// there gives the route on each group and each project of the workspace.
    extend Workspace as ProjectGroupHolder {
        /// The route `ListProjectGroups`, in this workspace.
        permission list_project_groups = read;
        /// The route `CreateProjectGroup`, in this workspace.
        permission create_project_group = work;
        /// The route `UpdateProjectGroup`, on each group of this workspace.
        permission update_project_group = work;
        /// The route `DeleteProjectGroup`, on each group of this workspace.
        permission delete_project_group = work;
        /// The route `MoveProject`, on each project of this workspace.
        permission move_project = work;
    }

    /// The routes of the order of the projects, on one project.
    extend Project as ProjectGroupAccess {
        /// The route `MoveProject`, on this project.
        permission move_project = edit;
    }
}

#[cfg(test)]
mod tests;
