//! The invitations into a workspace: a link to one email, with a role.

use time::OffsetDateTime;

use crate::identity::actors::ActorId;
use crate::tenancy::workspaces::WorkspaceId;
use crate::{Id, Page, PageRequest, ResourceKind};

/// The kind of an invitation, for the hooks.
pub const KIND: ResourceKind = ResourceKind::new("invitation");

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
    /// The actor that invited the person.
    pub invited_by: ActorId,
    /// The end of the validity of the link.
    pub expires_at: OffsetDateTime,
    /// The time of the last mail with a link.
    pub sent_at: OffsetDateTime,
}

/// One page of invitations, oldest first.
pub type InvitationPage = Page<Invitation, InvitationId>;

/// The input of a list of the open invitations of a workspace.
#[derive(Clone, Copy, Debug)]
pub struct ListInvitations {
    /// The workspace.
    pub workspace_id: WorkspaceId,
    /// The page to return.
    pub page: PageRequest<InvitationId>,
}
