//! The SQL of the people that a notice of a connection reaches: the owners of its workspace.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;

use crate::Error;

/// Returns the owners of a workspace.
pub(crate) async fn owners(tx: &mut Tx<'_>, workspace: WorkspaceId) -> Result<Vec<ActorId>, Error> {
    let users = sqlx::query_scalar!(
        "SELECT user_id FROM workspace_owners WHERE workspace_id = $1 ORDER BY user_id",
        workspace.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(users.into_iter().map(ActorId::from_db).collect())
}
