//! The SQL of the states of the installs of the GitHub App in `vcs_setup_states`. The table keeps
//! the hash of a state, never the state.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;
use time::OffsetDateTime;

use crate::Error;

/// Keeps the hash of a new state of an install, and deletes the states that expired one day ago
/// or more.
pub(crate) async fn insert(
    tx: &mut Tx<'_>,
    hash: &[u8],
    workspace: WorkspaceId,
    created_by: ActorId,
    expires_at: OffsetDateTime,
) -> Result<(), Error> {
    sqlx::query!(
        "DELETE FROM vcs_setup_states
         WHERE kind = 'github_install' AND expires_at < now() - interval '1 day'"
    )
    .execute(&mut **tx)
    .await?;
    sqlx::query!(
        "INSERT INTO vcs_setup_states (hash, kind, workspace_id, created_by, expires_at)
         VALUES ($1, 'github_install', $2, $3, $4)",
        hash,
        workspace.to_db(),
        created_by.to_db(),
        expires_at,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the workspace of a state of an install that is open for an actor: not used, not
/// expired, and made by this actor. Else `None`.
pub(crate) async fn open(
    tx: &mut Tx<'_>,
    hash: &[u8],
    actor: ActorId,
) -> Result<Option<WorkspaceId>, Error> {
    let workspace = sqlx::query_scalar!(
        r#"SELECT workspace_id AS "workspace_id!" FROM vcs_setup_states
           WHERE hash = $1 AND kind = 'github_install' AND created_by = $2 AND used_at IS NULL
             AND expires_at > now()"#,
        hash,
        actor.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(workspace.map(WorkspaceId::from_db))
}

/// Uses a state of an install that is open for an actor, one time. Returns its workspace, or
/// `None` when the state is not open.
pub(crate) async fn take(
    tx: &mut Tx<'_>,
    hash: &[u8],
    actor: ActorId,
) -> Result<Option<WorkspaceId>, Error> {
    let workspace = sqlx::query_scalar!(
        r#"UPDATE vcs_setup_states SET used_at = now()
           WHERE hash = $1 AND kind = 'github_install' AND created_by = $2 AND used_at IS NULL
             AND expires_at > now()
           RETURNING workspace_id AS "workspace_id!""#,
        hash,
        actor.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(workspace.map(WorkspaceId::from_db))
}
