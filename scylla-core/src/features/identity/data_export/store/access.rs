//! The reads of the access of a user for its export: the workspaces that it owns and its roles,
//! as JSON.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::Id;
use scylla_extension::Tx;
use serde_json::{json, Value};

use super::time;
use crate::Error;

/// Returns the workspaces that a user owns, by name.
pub(crate) async fn owned_workspaces(tx: &mut Tx<'_>, user: ActorId) -> Result<Vec<Value>, Error> {
    let rows = sqlx::query!(
        "SELECT w.id, w.name, w.slug, o.created_at
         FROM workspace_owners o
         JOIN workspaces w ON w.id = o.workspace_id
         WHERE o.user_id = $1
         ORDER BY w.name, w.id",
        user.to_db()
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({
                "id": WorkspaceId::from_db(row.id).to_string(),
                "name": row.name,
                "slug": row.slug,
                "owner_since": time(row.created_at),
            })
        })
        .collect())
}

/// Returns the roles of a user: the roles of the schema and the roles that are rows, with the
/// kind and the id of their scope, oldest first.
pub(crate) async fn roles(tx: &mut Tx<'_>, user: ActorId) -> Result<Vec<Value>, Error> {
    let rows = sqlx::query!(
        r#"SELECT scope_kind AS "scope_kind!", scope_id AS "scope_id!", role AS "role!",
                  created_at AS "created_at!"
           FROM (
               SELECT scope_kind, scope_id, relation AS role, created_at
               FROM relation_grants
               WHERE actor_id = $1
               UNION ALL
               SELECT g.scope_kind, g.scope_id, r.name AS role, g.created_at
               FROM role_grants g
               JOIN roles r ON r.id = g.role_id
               WHERE g.actor_id = $1
           ) AS held
           ORDER BY created_at, scope_kind, role"#,
        user.to_db()
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({
                "scope": row.scope_kind,
                "scope_id": Id::<()>::from_db(row.scope_id).to_string(),
                "role": row.role,
                "granted_at": time(row.created_at),
            })
        })
        .collect())
}
