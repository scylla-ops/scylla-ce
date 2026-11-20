//! The open items of the `apps` table: the enabled agents that are not connected.

use scylla_domain::identity::apps::AppId;
use scylla_domain::tenancy::dashboards::attention::{
    AttentionItem, AttentionKind, ItemAgent, Subject,
};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;
use time::OffsetDateTime;
use uuid::Uuid;

/// Returns the enabled agents of a workspace that the server last saw before `before`, except
/// the agents of `connected`. An agent that never connected is not in it.
pub(crate) async fn offline(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    connected: &[AppId],
    before: OffsetDateTime,
) -> Result<Vec<AttentionItem>, sqlx::Error> {
    let connected: Vec<Uuid> = connected.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query!(
        r#"SELECT id, name, last_seen_at AS "last_seen_at!" FROM apps
           WHERE workspace_id = $1 AND runner AND disabled_at IS NULL
             AND last_seen_at < $2 AND id <> ALL($3)"#,
        workspace.to_db(),
        before,
        &connected,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| AttentionItem {
            kind: AttentionKind::AgentOffline,
            at: row.last_seen_at,
            count: 1,
            is_new: false,
            place: None,
            subject: Subject::Agent(ItemAgent {
                id: AppId::from_db(row.id),
                name: row.name,
                last_seen_at: row.last_seen_at,
            }),
        })
        .collect())
}
