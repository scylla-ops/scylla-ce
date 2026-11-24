//! The SQL of the states of the manifest flow in `vcs_setup_states`. The table keeps the hash of a
//! state, never the state.

use scylla_domain::identity::actors::ActorId;
use scylla_extension::Tx;
use time::OffsetDateTime;

use crate::Error;

/// Keeps the hash of a new state of the manifest flow, and deletes the states of the manifest
/// that expired one day ago or more.
pub(crate) async fn insert(
    tx: &mut Tx<'_>,
    hash: &[u8],
    created_by: ActorId,
    expires_at: OffsetDateTime,
) -> Result<(), Error> {
    sqlx::query!(
        "DELETE FROM vcs_setup_states
         WHERE kind = 'github_manifest' AND expires_at < now() - interval '1 day'"
    )
    .execute(&mut **tx)
    .await?;
    sqlx::query!(
        "INSERT INTO vcs_setup_states (hash, kind, created_by, expires_at)
         VALUES ($1, 'github_manifest', $2, $3)",
        hash,
        created_by.to_db(),
        expires_at,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Uses a state of the manifest flow that is open for an actor, one time: not used, not expired,
/// and made by this actor. Returns `false` when the state is not open.
pub(crate) async fn take(tx: &mut Tx<'_>, hash: &[u8], actor: ActorId) -> Result<bool, Error> {
    let taken = sqlx::query!(
        "UPDATE vcs_setup_states SET used_at = now()
         WHERE hash = $1 AND kind = 'github_manifest' AND created_by = $2 AND used_at IS NULL
           AND expires_at > now()",
        hash,
        actor.to_db(),
    )
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(taken > 0)
}
