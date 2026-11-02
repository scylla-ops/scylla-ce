//! The SQL of the `invitations` table. Each function runs in the transaction of the caller.

use scylla_domain::actors::ActorId;
use scylla_domain::workspaces::{Invitation, InvitationId, WorkspaceId};
use scylla_extension::Tx;
use time::OffsetDateTime;

use crate::Error;

/// Inserts an invitation with the hash of its token.
pub(super) async fn insert(
    tx: &mut Tx<'_>,
    invitation: &Invitation,
    token_hash: &[u8],
    invited_by: ActorId,
) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO invitations (id, workspace_id, email, role, token_hash, invited_by, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        invitation.id.to_db(),
        invitation.workspace_id.to_db(),
        invitation.email,
        invitation.role,
        token_hash,
        invited_by.to_db(),
        invitation.expires_at,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the open invitation of a token hash: not accepted and not expired.
pub(super) async fn find_open(
    tx: &mut Tx<'_>,
    token_hash: &[u8],
) -> Result<Option<Invitation>, Error> {
    let row = sqlx::query!(
        "SELECT id, workspace_id, email, role, expires_at FROM invitations
         WHERE token_hash = $1 AND accepted_at IS NULL AND expires_at > now()",
        token_hash
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| Invitation {
        id: InvitationId::from_db(row.id),
        workspace_id: WorkspaceId::from_db(row.workspace_id),
        email: row.email,
        role: row.role,
        expires_at: row.expires_at,
    }))
}

/// Returns the oldest open invitation for an email, or `None`.
pub(super) async fn open_for_email(
    tx: &mut Tx<'_>,
    email: &str,
) -> Result<Option<Invitation>, Error> {
    let row = sqlx::query!(
        "SELECT id, workspace_id, email, role, expires_at FROM invitations
         WHERE email = $1 AND accepted_at IS NULL AND expires_at > now()
         ORDER BY id LIMIT 1",
        email
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| Invitation {
        id: InvitationId::from_db(row.id),
        workspace_id: WorkspaceId::from_db(row.workspace_id),
        email: row.email,
        role: row.role,
        expires_at: row.expires_at,
    }))
}

/// Marks an invitation as accepted by a user.
pub(super) async fn mark_accepted(
    tx: &mut Tx<'_>,
    id: InvitationId,
    user: ActorId,
) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE invitations SET accepted_by = $2, accepted_at = now() WHERE id = $1",
        id.to_db(),
        user.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the name of a workspace, for the mail.
pub(super) async fn workspace_name(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
) -> Result<Option<String>, Error> {
    let name = sqlx::query_scalar!(
        "SELECT name FROM workspaces WHERE id = $1",
        workspace.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(name)
}

/// The lifetime of an invitation link.
pub(super) fn expiry() -> OffsetDateTime {
    OffsetDateTime::now_utc() + time::Duration::days(7)
}
