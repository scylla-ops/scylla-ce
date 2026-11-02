//! The owners of a workspace: the users with each right in it, with no role.

use time::OffsetDateTime;

use crate::identity::actors::ActorId;
use crate::tenancy::workspaces::WorkspaceId;
use crate::{Page, PageRequest};

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
