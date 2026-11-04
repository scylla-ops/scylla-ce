//! The workspaces: the top-level containers. Each project and each agent is in one workspace.

use time::OffsetDateTime;

use crate::authz::Resource;
use crate::identity::actors::{Actor, ActorId};
use crate::tenancy::slugs::Slug;
use crate::{Id, Name, Page, PageRequest, ResourceKind};

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
    /// The unique slug, for the URLs.
    pub slug: String,
    /// The creation time, set by the database.
    pub created_at: OffsetDateTime,
    /// `true` when the personal tokens of the users act in this workspace.
    pub personal_tokens_enabled: bool,
}

/// How a new workspace gets its slug.
#[derive(Clone, Debug)]
pub enum NewSlug {
    /// A slug that the caller chose: a slug in use is refused.
    Chosen(Slug),
    /// A slug made from a name: a slug in use gets the first free suffix, `-2`, `-3` and on.
    Derived(Slug),
}

/// The input of a workspace creation.
#[derive(Clone, Debug)]
pub struct NewWorkspace {
    /// The user that creates the workspace. It becomes its first owner.
    pub creator: ActorId,
    /// The display name.
    pub name: WorkspaceName,
    /// The slug.
    pub slug: NewSlug,
}

/// A change of a workspace. A field that is `None` does not change.
#[derive(Clone, Debug, Default)]
pub struct WorkspaceChange {
    /// The new display name.
    pub name: Option<WorkspaceName>,
    /// The new slug. A slug in use is refused.
    pub slug: Option<Slug>,
    /// The new personal token switch.
    pub personal_tokens_enabled: Option<bool>,
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
        /// The route `UpdateWorkspace`: change the name and the slug.
        permission update_workspace = manage;
        /// The route `DeleteWorkspace`: delete an empty workspace. Only its owners: no role
        /// gives it.
        permission delete_workspace = nil;
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
        /// The route `ListAccess` of `AccessService`: the owners and the roles of this
        /// workspace, and the roles on each object in it.
        permission list_access = read;
        /// The route `CreateInvitation` of `InvitationService`: invite a person into this
        /// workspace.
        permission create_invitation = manage;
        /// The route `ListInvitations` of `InvitationService`: the open invitations.
        permission list_invitations = manage;
        /// The route `RevokeInvitation` of `InvitationService`: stop an open invitation.
        permission revoke_invitation = manage;
        /// The route `ResendInvitation` of `InvitationService`: send a new link.
        permission resend_invitation = manage;
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
