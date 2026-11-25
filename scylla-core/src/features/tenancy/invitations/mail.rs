//! The mail of an invitation: its link, with the names of the workspace, of the person who
//! invited, and of the role.

use scylla_domain::tenancy::invitations::Invitation;
use scylla_extension::Tx;

use crate::features::identity::accounts::{self, mails, AccountContext};
use crate::Error;

/// Reads the display name of the user who invited, or `None` for another actor.
pub(super) async fn inviter(
    tx: &mut Tx<'_>,
    invitation: &Invitation,
) -> Result<Option<String>, Error> {
    let user = accounts::find_user(tx, invitation.invited_by).await?;
    Ok(user.map(|user| user.display_name))
}

/// Puts the link of an invitation in the outbox, to its email only, in the transaction of the
/// invitation.
pub(super) async fn send(
    tx: &mut Tx<'_>,
    ctx: &AccountContext,
    invitation: &Invitation,
    workspace: &str,
    inviter: Option<String>,
    token: &str,
) -> Result<(), Error> {
    let (to, role) = (invitation.email.as_str(), invitation.role.as_str());
    let mail = mails::invitation(ctx, to, workspace, inviter.as_deref(), role, token);
    mails::queue(tx, ctx, "invitation", mail).await
}
