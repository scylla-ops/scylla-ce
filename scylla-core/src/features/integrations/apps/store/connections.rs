//! The SQL of the connections of the GitHub App of the instance in `vcs_connections`, when an
//! admin removes the App.

use scylla_domain::integrations::apps::VcsAppId;
use scylla_domain::integrations::connections::VcsConnectionId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;

use crate::features::integrations::connections::store::health::Changed;
use crate::features::integrations::connections::store::installations::StateChange;
use crate::Error;

/// Moves each connection of an App to `disconnected` with a reason, and takes the App off it, so
/// that the delete of the App keeps the connection. Returns the connections, with their state
/// before.
pub(crate) async fn disconnect(
    tx: &mut Tx<'_>,
    app: VcsAppId,
    reason: &str,
) -> Result<Vec<StateChange>, Error> {
    let rows = sqlx::query!(
        r#"UPDATE vcs_connections c
           SET state = 'disconnected', state_reason = $2, app_id = NULL, updated_at = now()
           FROM (SELECT id, state FROM vcs_connections WHERE app_id = $1 FOR UPDATE) old
           WHERE c.id = old.id
           RETURNING c.id, c.workspace_id, c.provider, c.account, c.token_expires_at,
                     old.state AS "from_state!""#,
        app.to_db(),
        reason,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| StateChange {
            changed: Changed {
                id: VcsConnectionId::from_db(row.id),
                workspace: WorkspaceId::from_db(row.workspace_id),
                provider: row.provider,
                account: row.account,
                expires_at: row.token_expires_at,
            },
            from: row.from_state,
        })
        .collect())
}
