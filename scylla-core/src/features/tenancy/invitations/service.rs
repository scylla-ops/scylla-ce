//! The logic of the invitations: a link by mail that gives a role in a workspace to the person
//! with the email, and the management of the open invitations.

use scylla_domain::authz::Resource;
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::users::{self, Email};
use scylla_domain::tenancy::grants::MEMBER_KIND;
use scylla_domain::tenancy::invitations::{
    self, Invitation, InvitationId, InvitationPage, ListInvitations,
};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_domain::Page;
use scylla_extension::{CreateContext, Grantable, Rejection, Tx};
use sqlx::PgPool;

use super::store;
use crate::features::identity::accounts::{mails, refused, verified_emails, AccountContext};
use crate::features::notifications::{self, kinds};
use crate::features::tenancy::grants;
use crate::infra::logs::mask_email;
use crate::infra::secret;
use crate::Error;

/// Which open invitation an acceptance takes.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Pick<'a> {
    /// The invitation of the token of a link.
    Token(&'a str),
    /// An invitation found by its email.
    Id(InvitationId),
}

/// Creates an invitation and sends its link to the email. The role must be a role of the
/// workspaces in the registry of the edition. An open invitation of the email in the workspace
/// gives `INVITATION_PENDING`.
pub(crate) async fn create(
    ctx: &AccountContext,
    inviter: Actor,
    workspace: WorkspaceId,
    email: Email,
    role: &str,
) -> Result<Invitation, Error> {
    // Refuse a name that is not a role of the workspaces.
    let invited_by = inviter.id;
    let id = workspace.to_string();
    if workspace_roles(ctx)?
        .relationship(role, &id, invited_by)
        .is_none()
    {
        return Err(Error::invalid("role", "is not a role of the workspaces"));
    }

    // Store the invitation with the hash of a new token.
    let token = secret::token();
    let mut tx = ctx.pool.begin().await?;
    let name = store::workspace_name(&mut tx, workspace)
        .await?
        .ok_or(Error::NotFound("workspace"))?;
    let hook = CreateContext::new(invitations::KIND, inviter, workspace);
    ctx.hooks.before_create(&mut tx, &hook).await?;
    let hash = secret::hash(&token);
    let invitation =
        store::insert(&mut tx, workspace, email.as_str(), role, invited_by, &hash).await?;
    let fact = kinds::invitation_received(&invitation, None);
    notifications::emit(&mut tx, fact).await?;
    tx.commit().await?;
    let email = mask_email(&invitation.email);
    let (id, role) = (invitation.id, &invitation.role);
    tracing::info!(event = "invitation.created", invitation = %id, %workspace, email, role);

    // Send the link to the email only.
    let mail = mails::invitation(ctx, &invitation.email, &name, &token);
    mails::send(ctx, "invitation", mail).await;
    Ok(invitation)
}

/// Returns `true` if a workspace exists.
pub(super) async fn workspace_exists(pool: &PgPool, workspace: WorkspaceId) -> Result<bool, Error> {
    let mut tx = pool.begin().await?;
    let name = store::workspace_name(&mut tx, workspace).await?;
    tx.commit().await?;
    Ok(name.is_some())
}

/// Returns one page of the invitations of a workspace that nobody accepted, oldest first.
pub(super) async fn list(pool: &PgPool, query: ListInvitations) -> Result<InvitationPage, Error> {
    let mut tx = pool.begin().await?;
    let invitations = store::list_open(&mut tx, query.workspace_id, query.page).await?;
    tx.commit().await?;
    Ok(Page::from_overfetch(
        invitations,
        query.page.size,
        |invitation| invitation.id,
    ))
}

/// Deletes an open invitation of a workspace, so its link stops. An invitation that is not open
/// in the workspace gives `NotFound("invitation")`.
pub(super) async fn revoke(
    pool: &PgPool,
    workspace: WorkspaceId,
    id: InvitationId,
) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    if !store::delete_open(&mut tx, workspace, id).await? {
        return Err(Error::NotFound("invitation"));
    }
    notifications::withdraw(&mut tx, invitations::KIND.as_str(), id.to_db()).await?;
    tx.commit().await?;
    tracing::info!(event = "invitation.revoked", invitation = %id, %workspace);
    Ok(())
}

/// Gives an open invitation a new token and a new expiry, and sends the new link. The old link
/// stops. A second send in less than 60 seconds gives `RESEND_TOO_SOON`.
pub(super) async fn resend(
    ctx: &AccountContext,
    workspace: WorkspaceId,
    id: InvitationId,
) -> Result<Invitation, Error> {
    // Lock the invitation, and refuse a send soon after the last one.
    let mut tx = ctx.pool.begin().await?;
    let (_, wait) = store::lock_for_resend(&mut tx, workspace, id)
        .await?
        .ok_or(Error::NotFound("invitation"))?;
    if let Some(seconds) = wait {
        let message = "the invitation got a mail less than 60 seconds ago";
        let rejection =
            Rejection::new("RESEND_TOO_SOON", message).detail("retry_after_seconds", seconds);
        return Err(Error::Rejected(rejection));
    }

    // Store the hash of a new token, then send the new link to the email only.
    let token = secret::token();
    let invitation = store::renew(&mut tx, id, &secret::hash(&token)).await?;
    let kind = invitations::KIND.as_str();
    notifications::reopen(&mut tx, kinds::INVITATION_RECEIVED, kind, id.to_db()).await?;
    let name = store::workspace_name(&mut tx, workspace)
        .await?
        .ok_or(Error::NotFound("workspace"))?;
    tx.commit().await?;
    tracing::info!(event = "invitation.resent", invitation = %id, %workspace);
    let mail = mails::invitation(ctx, &invitation.email, &name, &token);
    mails::send(ctx, "invitation", mail).await;
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

/// Records the fact of each open invitation of an email, for a user that just got this email as a
/// verified email.
pub(crate) async fn announce_open(
    tx: &mut Tx<'_>,
    user: ActorId,
    email: &str,
) -> Result<(), Error> {
    for invitation in store::all_open_for_email(tx, email).await? {
        notifications::emit(tx, kinds::invitation_received(&invitation, Some(user))).await?;
    }
    Ok(())
}

/// Returns the user with the verified email of an open invitation.
pub(crate) async fn invitee(pool: &PgPool, id: InvitationId) -> Result<Option<ActorId>, Error> {
    let mut tx = pool.begin().await?;
    let user = store::invitee(&mut tx, id).await?;
    tx.commit().await?;
    Ok(user)
}

/// Gives the role of an open invitation to a user with its verified email, and returns the
/// workspace. One transaction locks the invitation, writes the role and marks the invitation,
/// so a revoke or a resend waits, or wins and the link gives the answer of an unknown link. A
/// retry after a failed commit writes the same role again with no harm.
pub(crate) async fn accept(
    ctx: &AccountContext,
    pick: Pick<'_>,
    user: ActorId,
) -> Result<WorkspaceId, Error> {
    // Lock the open invitation.
    let mut tx = ctx.pool.begin().await?;
    let invitation = match pick {
        Pick::Token(token) => {
            store::lock_acceptable(&mut tx, Some(&secret::hash(token)), None).await?
        }
        Pick::Id(id) => store::lock_acceptable(&mut tx, None, Some(id)).await?,
    };
    let invitation = invitation.ok_or_else(|| not_open(pick))?;
    check_email(&mut tx, user, &invitation).await?;

    // Mark the invitation, let the hooks check the new member, give the role, and commit.
    let id = invitation.workspace_id.to_string();
    let role = workspace_roles(ctx)?
        .relationship(&invitation.role, &id, user)
        .ok_or_else(|| Error::Internal(format!("unknown role {}", invitation.role)))?;
    if !store::mark_accepted(&mut tx, invitation.id, user).await? {
        return Err(not_open(pick));
    }
    let member = Actor {
        id: user,
        kind: users::KIND,
    };
    let object = Some(format!("{}:{id}", Workspace::NAME));
    let hook = CreateContext::new(MEMBER_KIND, member, invitation.workspace_id).with_member(
        user,
        object,
        &invitation.role,
    );
    ctx.hooks.before_create(&mut tx, &hook).await?;
    ctx.gate.write(&[role]).await?;
    let scope = (Workspace::NAME, invitation.workspace_id.to_db());
    let by = invitation.invited_by;
    grants::store::note_relation(&mut tx, user, scope, &invitation.role, by).await?;
    let fact = kinds::invitation_answered(kinds::INVITATION_ACCEPTED, &invitation, user);
    notifications::emit(&mut tx, fact).await?;
    let kind = invitations::KIND.as_str();
    notifications::close(
        &mut tx,
        kinds::INVITATION_RECEIVED,
        kind,
        invitation.id.to_db(),
    )
    .await?;
    tx.commit().await?;
    let (id, workspace, role) = (invitation.id, invitation.workspace_id, &invitation.role);
    tracing::info!(event = "invitation.accepted", invitation = %id, %workspace, %user, role);
    Ok(invitation.workspace_id)
}

/// Declines an open invitation for a user with its verified email: the invitation goes, and the
/// person that invited gets a notification.
pub(super) async fn decline(
    ctx: &AccountContext,
    id: InvitationId,
    user: ActorId,
) -> Result<(), Error> {
    // Lock the open invitation, and refuse a user with no verified email of it.
    let mut tx = ctx.pool.begin().await?;
    let invitation = store::lock_acceptable(&mut tx, None, Some(id))
        .await?
        .ok_or(Error::NotFound("invitation"))?;
    check_email(&mut tx, user, &invitation).await?;

    // Delete the invitation and its rows, then tell the person that invited.
    let workspace = invitation.workspace_id;
    if !store::delete_open(&mut tx, workspace, id).await? {
        return Err(Error::NotFound("invitation"));
    }
    notifications::withdraw(&mut tx, invitations::KIND.as_str(), id.to_db()).await?;
    let fact = kinds::invitation_answered(kinds::INVITATION_DECLINED, &invitation, user);
    notifications::emit(&mut tx, fact).await?;
    tx.commit().await?;
    tracing::info!(event = "invitation.declined", invitation = %id, %workspace, %user);
    Ok(())
}

/// Refuses a user with no verified email of an invitation.
async fn check_email(tx: &mut Tx<'_>, user: ActorId, invitation: &Invitation) -> Result<(), Error> {
    if verified_emails(tx, user).await?.contains(&invitation.email) {
        return Ok(());
    }
    let message = "the invitation is for another email";
    Err(refused("INVITATION_FOR_ANOTHER_EMAIL", message))
}

/// The answer to an invitation that is not open: unknown, stopped, accepted or expired. For a link
/// it is the answer of an unknown link.
fn not_open(pick: Pick<'_>) -> Error {
    match pick {
        Pick::Token(_) => Error::invalid("token", "is not an open invitation"),
        Pick::Id(_) => Error::NotFound("invitation"),
    }
}

/// Returns the roles of the workspaces in the registry of the edition.
fn workspace_roles(ctx: &AccountContext) -> Result<&Grantable, Error> {
    ctx.grants
        .get(Workspace::NAME)
        .ok_or_else(|| Error::Internal("the workspaces have no roles".to_owned()))
}
