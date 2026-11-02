//! The invitations into a workspace: a link to one email, with a role.

use time::OffsetDateTime;

use crate::tenancy::workspaces::WorkspaceId;
use crate::Id;

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
