//! The SQL of a list of the access: the owners of a workspace and the profiles of the actors.

use scylla_domain::identity::actors::{ActorId, ActorKind};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::grants::{AccessEntry, OWNER};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::Error;

/// Returns at most `limit` owners of a workspace after a user, in user id order.
pub(crate) async fn owners(
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

/// Returns the name of a workspace, or an empty text for an unknown workspace.
pub(crate) async fn workspace_name(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
) -> Result<String, Error> {
    let name = sqlx::query_scalar!(
        "SELECT name FROM workspaces WHERE id = $1",
        workspace.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(name.unwrap_or_default())
}

/// Returns the projects of a workspace with their names.
pub(crate) async fn projects_of(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
) -> Result<Vec<(ProjectId, String)>, Error> {
    let rows = sqlx::query!(
        "SELECT id, name FROM projects WHERE workspace_id = $1 ORDER BY lower(name)",
        workspace.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (ProjectId::from_db(row.id), row.name))
        .collect())
}

/// Returns the owners of a workspace, with the time that each one became an owner.
pub(crate) async fn owners_since(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
) -> Result<Vec<(ActorId, OffsetDateTime)>, Error> {
    let rows = sqlx::query!(
        "SELECT user_id, created_at FROM workspace_owners WHERE workspace_id = $1",
        workspace.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (ActorId::from_db(row.user_id), row.created_at))
        .collect())
}

/// Returns the time that an actor became an owner of a workspace, if it is an owner.
pub(crate) async fn owner_since(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    actor: ActorId,
) -> Result<Option<OffsetDateTime>, Error> {
    let since = sqlx::query_scalar!(
        "SELECT created_at FROM workspace_owners WHERE workspace_id = $1 AND user_id = $2",
        workspace.to_db(),
        actor.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(since)
}

/// Returns the owners of all the workspaces, with each workspace and the time.
pub(crate) async fn all_owners(
    tx: &mut Tx<'_>,
) -> Result<Vec<(ActorId, WorkspaceId, String, OffsetDateTime)>, Error> {
    let rows = sqlx::query!(
        "SELECT o.user_id, o.workspace_id, w.name, o.created_at FROM workspace_owners o
         JOIN workspaces w ON w.id = o.workspace_id",
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let workspace = WorkspaceId::from_db(row.workspace_id);
            (
                ActorId::from_db(row.user_id),
                workspace,
                row.name,
                row.created_at,
            )
        })
        .collect())
}

/// Returns the instance admins, with the time that each one became an admin.
pub(crate) async fn admins_since(tx: &mut Tx<'_>) -> Result<Vec<(ActorId, OffsetDateTime)>, Error> {
    let rows = sqlx::query!("SELECT user_id, created_at FROM instance_admins")
        .fetch_all(&mut **tx)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| (ActorId::from_db(row.user_id), row.created_at))
        .collect())
}

/// Returns `true` if an actor other than the user has a role that is a row on a scope.
pub(crate) async fn others_on_scope(
    tx: &mut Tx<'_>,
    scope: Uuid,
    user: ActorId,
) -> Result<bool, Error> {
    let found = sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM role_grants WHERE scope_id = $1 AND actor_id <> $2
           ) AS "found!""#,
        scope,
        user.to_db(),
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(found)
}

/// Returns the kind of each actor, with the display name and the handle of a user, or the name
/// of an App. The role of each entry is empty.
pub(crate) async fn profiles(
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
