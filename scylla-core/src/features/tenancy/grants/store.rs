//! The SQL of a list of the access: the owners of a workspace and the profiles of the actors.

use scylla_domain::identity::actors::{ActorId, ActorKind};
use scylla_domain::tenancy::grants::{AccessEntry, OWNER};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;

use crate::Error;

/// Returns at most `limit` owners of a workspace after a user, in user id order.
pub(super) async fn owners(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    after: Option<ActorId>,
    limit: usize,
) -> Result<Vec<AccessEntry>, Error> {
    let rows = sqlx::query!(
        r#"SELECT o.user_id, a.kind, u.display_name, u.handle FROM workspace_owners o
           JOIN users u ON u.id = o.user_id
           JOIN actors a ON a.id = o.user_id
           WHERE o.workspace_id = $1 AND ($2::uuid IS NULL OR o.user_id > $2)
           ORDER BY o.user_id
           LIMIT $3"#,
        workspace.to_db(),
        after.map(ActorId::to_db),
        limit as i64,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| AccessEntry {
            actor_id: ActorId::from_db(row.user_id),
            actor_kind: ActorKind::from_name(row.kind),
            display_name: row.display_name,
            handle: row.handle,
            role: OWNER.to_owned(),
        })
        .collect())
}

/// Returns the kind of each actor, with the display name and the handle of a user, or the name
/// of an App. The role of each entry is empty.
pub(super) async fn profiles(
    tx: &mut Tx<'_>,
    actors: &[ActorId],
) -> Result<Vec<AccessEntry>, Error> {
    let ids: Vec<_> = actors.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query!(
        r#"SELECT a.id AS "id!", a.kind AS "kind!",
                  coalesce(u.display_name, p.name) AS "display_name?", u.handle
           FROM actors a
           LEFT JOIN users u ON u.id = a.id
           LEFT JOIN apps p ON p.id = a.id
           WHERE a.id = ANY($1)"#,
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| AccessEntry {
            actor_id: ActorId::from_db(row.id),
            actor_kind: ActorKind::from_name(row.kind),
            display_name: row.display_name.unwrap_or_default(),
            handle: row.handle,
            role: String::new(),
        })
        .collect())
}
