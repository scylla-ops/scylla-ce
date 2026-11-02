//! The logic of the invitations: a link by mail that gives a role in a workspace to the person
//! with the email.

use scylla_domain::authz::Resource;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::users::Email;
use scylla_domain::tenancy::invitations::{Invitation, InvitationId};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_extension::{Grantable, Tx};
use sqlx::PgPool;

use super::store;
use crate::features::identity::accounts::{mails, AccountContext};
use crate::infra::secret;
use crate::Error;

/// Creates an invitation and sends its link to the email. The role must be a role of the
/// workspaces in the registry of the edition.
pub(crate) async fn create(
    ctx: &AccountContext,
    invited_by: ActorId,
    workspace: WorkspaceId,
    email: Email,
    role: &str,
) -> Result<Invitation, Error> {
    // Refuse a name that is not a role of the workspaces.
    let id = workspace.to_string();
    if workspace_roles(ctx)?
        .relationship(role, &id, invited_by)
        .is_none()
    {
        return Err(Error::invalid("role", "is not a role of the workspaces"));
    }

    // Store the invitation with the hash of a new token.
    let token = secret::token();
    let invitation = Invitation {
        id: InvitationId::generate(),
        workspace_id: workspace,
        email: email.as_str().to_owned(),
        role: role.to_owned(),
        expires_at: store::expiry(),
    };
    let mut tx = ctx.pool.begin().await?;
    let name = store::workspace_name(&mut tx, workspace)
        .await?
        .ok_or(Error::NotFound("workspace"))?;
    store::insert(&mut tx, &invitation, &secret::hash(&token), invited_by).await?;
    tx.commit().await?;

    // Send the link to the email only.
    let mail = mails::invitation(ctx, &invitation.email, &name, &token);
    mails::send(ctx, mail).await;
    Ok(invitation)
}

/// Returns the open invitation of a token, or `None`.
pub(crate) async fn find_open(pool: &PgPool, token: &str) -> Result<Option<Invitation>, Error> {
    let mut tx = pool.begin().await?;
    let invitation = store::find_open(&mut tx, &secret::hash(token)).await?;
    tx.commit().await?;
    Ok(invitation)
}

/// Returns the oldest open invitation for an email, in the transaction of the caller.
pub(crate) async fn open_for_email(
    tx: &mut Tx<'_>,
    email: &str,
) -> Result<Option<Invitation>, Error> {
    store::open_for_email(tx, email).await
}

/// Gives the role of an invitation to a user, then marks the invitation as accepted. The role
/// is a SpiceDB write, so it comes first: a retry writes it again with no harm.
pub(crate) async fn accept(
    ctx: &AccountContext,
    invitation: &Invitation,
    user: ActorId,
) -> Result<(), Error> {
    let id = invitation.workspace_id.to_string();
    let role = workspace_roles(ctx)?
        .relationship(&invitation.role, &id, user)
        .ok_or_else(|| Error::Internal(format!("unknown role {}", invitation.role)))?;
    ctx.gate.write(&[role]).await?;
    let mut tx = ctx.pool.begin().await?;
    store::mark_accepted(&mut tx, invitation.id, user).await?;
    tx.commit().await?;
    Ok(())
}

/// Returns the roles of the workspaces in the registry of the edition.
fn workspace_roles(ctx: &AccountContext) -> Result<&Grantable, Error> {
    ctx.grants
        .get(Workspace::NAME)
        .ok_or_else(|| Error::Internal("the workspaces have no roles".to_owned()))
}
