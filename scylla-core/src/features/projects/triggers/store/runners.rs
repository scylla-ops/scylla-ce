//! The SQL of the `trigger_runners` table: the actor that authors the jobs of the triggers of a
//! workspace.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;

/// Returns the runner of a workspace, or `None` if the workspace has none yet.
pub(crate) async fn find(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
) -> Result<Option<ActorId>, sqlx::Error> {
    let actor = sqlx::query_scalar!(
        "SELECT actor_id FROM trigger_runners WHERE workspace_id = $1",
        workspace.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(actor.map(ActorId::from_db))
}

/// Records the runner of a workspace, unless the workspace has one. Returns `false` if the
/// workspace already has one.
pub(crate) async fn insert(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    actor: ActorId,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        "INSERT INTO trigger_runners (workspace_id, actor_id) VALUES ($1, $2)
         ON CONFLICT (workspace_id) DO NOTHING",
        workspace.to_db(),
        actor.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected() == 1)
}
