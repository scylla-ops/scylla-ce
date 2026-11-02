//! The SQL of the gate: the token revocations and the owners of the workspaces.

use scylla_domain::actors::ActorId;
use scylla_domain::users::SessionId;
use scylla_domain::workspaces::WorkspaceId;
use scylla_extension::{AccessError, Tx};

/// Returns `true` if a token cannot pass: one of its revocation ids is revoked, its session is
/// revoked or expired, or its user is disabled.
pub(super) async fn is_refused(
    tx: &mut Tx<'_>,
    ids: &[Vec<u8>],
    actor: ActorId,
    session: Option<SessionId>,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT (
               EXISTS (SELECT 1 FROM revoked_tokens WHERE revocation_id = ANY($1))
               OR EXISTS (SELECT 1 FROM users WHERE id = $2 AND status = 'disabled')
               OR ($3::uuid IS NOT NULL AND NOT EXISTS (
                   SELECT 1 FROM sessions
                   WHERE id = $3 AND revoked_at IS NULL AND expires_at > now()))
           ) AS "refused!""#,
        ids,
        actor.to_db(),
        session.map(SessionId::to_db),
    )
    .fetch_one(&mut **tx)
    .await
}

/// Revokes a revocation id. A token that has it never passes again.
pub async fn revoke(tx: &mut Tx<'_>, id: &[u8]) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO revoked_tokens (revocation_id) VALUES ($1) ON CONFLICT DO NOTHING",
        id,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns `true` if the actor is one of the owners of the workspace. An unknown workspace has
/// no owner.
pub(super) async fn is_owner(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    actor: ActorId,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM workspace_owners WHERE workspace_id = $1 AND user_id = $2
           ) AS "owner!""#,
        workspace.to_db(),
        actor.to_db(),
    )
    .fetch_one(&mut **tx)
    .await
}

/// Logs a database failure of the gate, and hides its details from the client.
pub(super) fn failed(error: sqlx::Error) -> AccessError {
    tracing::error!(%error, "the access store failed");
    AccessError::Unavailable("access check unavailable".to_owned())
}
