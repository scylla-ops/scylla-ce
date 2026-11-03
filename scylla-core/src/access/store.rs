//! The SQL of the gate: the token revocations, the owners of the workspaces and the instance
//! admins.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::apps::AppKeyId;
use scylla_domain::identity::users::SessionId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{AccessError, Tx};

/// The facts of a token that can stop it.
pub(super) struct TokenFacts<'a> {
    /// The revocation ids of the token.
    pub(super) ids: &'a [Vec<u8>],
    /// The actor of the token.
    pub(super) actor: ActorId,
    /// `true` if the actor is a user: it must have a row in `users`.
    pub(super) user: bool,
    /// `true` if the actor is an App: it must have a row in `apps`.
    pub(super) app: bool,
    /// The session of the token.
    pub(super) session: Option<SessionId>,
    /// The App key of the token.
    pub(super) key: Option<AppKeyId>,
}

/// Returns `true` if a token cannot pass: one of its revocation ids is revoked, its session is
/// revoked or expired, its user is disabled or deleted, its App is deleted, or its key is
/// revoked.
pub(super) async fn is_refused(
    tx: &mut Tx<'_>,
    facts: &TokenFacts<'_>,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT (
               EXISTS (SELECT 1 FROM revoked_tokens WHERE revocation_id = ANY($1))
               OR EXISTS (SELECT 1 FROM users WHERE id = $2 AND status = 'disabled')
               OR ($3 AND NOT EXISTS (SELECT 1 FROM users WHERE id = $2))
               OR ($4::uuid IS NOT NULL AND NOT EXISTS (
                   SELECT 1 FROM sessions
                   WHERE id = $4 AND revoked_at IS NULL AND expires_at > now()))
               OR ($5 AND NOT EXISTS (SELECT 1 FROM apps WHERE id = $2))
               OR ($6::uuid IS NOT NULL AND NOT EXISTS (
                   SELECT 1 FROM app_keys
                   WHERE id = $6 AND app_id = $2 AND revoked_at IS NULL))
           ) AS "refused!""#,
        facts.ids,
        facts.actor.to_db(),
        facts.user,
        facts.session.map(SessionId::to_db),
        facts.app,
        facts.key.map(AppKeyId::to_db),
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

/// Returns `true` if the actor is an admin of the instance.
pub(super) async fn is_admin(tx: &mut Tx<'_>, actor: ActorId) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM instance_admins WHERE user_id = $1) AS "admin!""#,
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
