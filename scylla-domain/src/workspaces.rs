//! The workspaces: the top-level containers. Each project and each agent is in one workspace.

use time::OffsetDateTime;

use crate::actors::{Actor, ActorId};
use crate::authz::Resource;
use crate::name::Name;
use crate::page::{Page, PageRequest};
use crate::{Id, ResourceKind};

/// The kind of a workspace, for the hooks: the name of its definition.
pub const KIND: ResourceKind = ResourceKind::new(Workspace::NAME);

/// The id of a workspace.
pub type WorkspaceId = Id<Workspace>;

/// The display name of a workspace.
pub type WorkspaceName = Name;

/// A workspace, as the store returns it. Its owners are rows of their own.
#[derive(Clone, Debug)]
pub struct Workspace {
    /// The id of the workspace.
    pub id: WorkspaceId,
    /// The display name.
    pub name: String,
    /// The creation time, set by the database.
    pub created_at: OffsetDateTime,
}

/// The input of a workspace creation.
#[derive(Clone, Debug)]
pub struct NewWorkspace {
    /// The user that creates the workspace. It becomes its first owner.
    pub creator: ActorId,
    /// The display name.
    pub name: WorkspaceName,
}

/// What a caller can read: the workspaces that it owns, and the workspaces where it has a role.
#[derive(Clone, Debug)]
pub struct VisibleWorkspaces {
    /// The caller: the workspaces that it owns are visible.
    pub caller: ActorId,
    /// The workspaces where the caller has a role.
    pub granted: Vec<WorkspaceId>,
}

/// One page of workspaces.
pub type WorkspacePage = Page<Workspace, WorkspaceId>;

/// The input of a workspace list.
#[derive(Clone, Copy, Debug)]
pub struct ListWorkspaces {
    /// The page to return.
    pub page: PageRequest<WorkspaceId>,
}

/// An owner of a workspace: a user with each right in it, with no role. A workspace has one
/// owner or more.
#[derive(Clone, Debug)]
pub struct WorkspaceOwner {
    /// The user.
    pub user_id: ActorId,
    /// The display name of the user.
    pub display_name: String,
    /// When the user became an owner.
    pub since: OffsetDateTime,
}

/// One page of owners, in user id order.
pub type OwnerPage = Page<WorkspaceOwner, ActorId>;

/// The input of an owner list.
#[derive(Clone, Copy, Debug)]
pub struct ListOwners {
    /// The workspace.
    pub workspace_id: WorkspaceId,
    /// The page to return.
    pub page: PageRequest<ActorId>,
}

crate::authz! {
    /// The roles in a workspace. Its owners are rows of `workspace_owners` in Postgres, not
    /// here. `manage`, `work` and `read` are groups, and each route has its own permission.
    definition workspace for Workspace {
        /// An actor that manages the workspace: its roles and its settings.
        relation admin: Actor;
        /// An actor that works in the workspace: it creates and changes projects.
        relation member: Actor;
        /// An actor that reads the workspace and its projects.
        relation viewer: Actor;
        /// The group of the routes that manage the workspace.
        permission manage = admin;
        /// The group of the routes that work in the workspace.
        permission work = manage + member;
        /// The group of the routes that read the workspace.
        permission read = work + viewer;
        /// The route `GetWorkspace`.
        permission get_workspace = read;
        /// The route `ListWorkspaces`: a workspace that the caller can read is in its list.
        permission list_workspaces = read;
        /// The route `AddOwner`: make a user an owner of this workspace. Only its owners: no
        /// role gives it.
        permission add_owner = nil;
        /// The route `RemoveOwner`: take an owner out of this workspace. Only its owners.
        permission remove_owner = nil;
        /// The route `ListOwners`: the owners of this workspace.
        permission list_owners = read;
        /// The route `Grant` of `AccessService`: give a role in this workspace. Only its
        /// admins.
        permission grant = admin;
        /// The route `Revoke` of `AccessService`: take back a role in this workspace.
        permission revoke = admin;
        /// The route `CreateInvitation` of `InvitationService`: invite a person into this
        /// workspace.
        permission create_invitation = manage;
    }

    /// The routes of the workspaces on the caller itself.
    extend Actor as WorkspaceCaller {
        /// The route `CreateWorkspace`: a user creates a workspace and becomes its first owner.
        permission create_workspace = nil;
        /// The route `ListWorkspaces`: each caller lists the workspaces that it can read.
        permission list_workspaces = nil;
        /// The route `AcceptInvitation` of `InvitationService`: a user accepts an invitation for
        /// its email.
        permission accept_invitation = nil;
    }
}

/// The id of an invitation.
pub type InvitationId = Id<Invitation>;

/// An invitation into a workspace, as the store returns it.
#[derive(Clone, Debug)]
pub struct Invitation {
    /// The id of the invitation.
    pub id: InvitationId,
    /// The workspace.
    pub workspace_id: WorkspaceId,
    /// The email of the invited person, in lower case.
    pub email: String,
    /// The role to give in the workspace.
    pub role: String,
    /// The end of the validity of the link.
    pub expires_at: OffsetDateTime,
}
