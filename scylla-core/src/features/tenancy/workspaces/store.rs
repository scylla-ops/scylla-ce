//! The SQL of the `workspaces` and `workspace_owners` tables. Each function runs in the
//! transaction of the caller.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::owners::WorkspaceOwner;
use scylla_domain::tenancy::workspaces::{NewWorkspace, VisibleWorkspaces, Workspace, WorkspaceId};
use scylla_domain::PageRequest;
use scylla_extension::Tx;

use crate::Error;

/// The foreign key from an owner to its user.
const USER_FOREIGN_KEY: &str = "workspace_owners_user_id_fkey";

/// Inserts a workspace and its first owner, and returns the workspace.
pub(crate) async fn insert(tx: &mut Tx<'_>, new: &NewWorkspace) -> Result<Workspace, Error> {
    let id = WorkspaceId::generate();
    let created_at = sqlx::query_scalar!(
        "INSERT INTO workspaces (id, name) VALUES ($1, $2) RETURNING created_at",
        id.to_db(),
        new.name.as_str(),
    )
    .fetch_one(&mut **tx)
    .await?;
    add_owner(tx, id, new.creator).await?;
    Ok(Workspace {
        id,
        name: new.name.as_str().to_owned(),
        created_at,
    })
}

/// Returns a workspace, or `None` if no workspace has this id.
pub(super) async fn find(tx: &mut Tx<'_>, id: WorkspaceId) -> Result<Option<Workspace>, Error> {
    let row = sqlx::query!(
        "SELECT id, name, created_at FROM workspaces WHERE id = $1",
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| Workspace {
        id: WorkspaceId::from_db(row.id),
        name: row.name,
        created_at: row.created_at,
    }))
}

/// Returns the workspaces that a caller can read, after the page id, in id order, with one
/// extra row.
pub(super) async fn list_visible(
    tx: &mut Tx<'_>,
    visible: &VisibleWorkspaces,
    page: PageRequest<WorkspaceId>,
) -> Result<Vec<Workspace>, Error> {
    let granted: Vec<_> = visible.granted.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query!(
        "SELECT id, name, created_at FROM workspaces
         WHERE (id IN (SELECT workspace_id FROM workspace_owners WHERE user_id = $1)
                OR id = ANY($2))
           AND ($3::uuid IS NULL OR id > $3)
         ORDER BY id
         LIMIT $4",
        visible.caller.to_db(),
        &granted,
        page.after.map(WorkspaceId::to_db),
        page.size.get() as i64 + 1,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Workspace {
            id: WorkspaceId::from_db(row.id),
            name: row.name,
            created_at: row.created_at,
        })
        .collect())
}

/// Locks the row of a workspace until the end of the transaction. Returns `false` if no
/// workspace has this id.
pub(super) async fn lock(tx: &mut Tx<'_>, id: WorkspaceId) -> Result<bool, Error> {
    let row = sqlx::query_scalar!(
        "SELECT id FROM workspaces WHERE id = $1 FOR UPDATE",
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.is_some())
}

/// Makes a user an owner of a workspace. An owner stays an owner. An unknown user gives
/// `NotFound`.
pub(super) async fn add_owner(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    user: ActorId,
) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO workspace_owners (workspace_id, user_id) VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
        workspace.to_db(),
        user.to_db(),
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(db) if db.constraint() == Some(USER_FOREIGN_KEY) => {
            Error::NotFound("user")
        }
        error => error.into(),
    })?;
    Ok(())
}

/// Takes a user out of the owners of a workspace. A user that is not an owner changes nothing.
pub(super) async fn remove_owner(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    user: ActorId,
) -> Result<(), Error> {
    sqlx::query!(
        "DELETE FROM workspace_owners WHERE workspace_id = $1 AND user_id = $2",
        workspace.to_db(),
        user.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Counts the owners of a workspace.
pub(super) async fn count_owners(tx: &mut Tx<'_>, workspace: WorkspaceId) -> Result<i64, Error> {
    let count = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM workspace_owners WHERE workspace_id = $1"#,
        workspace.to_db()
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(count)
}

/// Returns the owners of a workspace, after the page id, in user id order, with one extra row.
pub(super) async fn owners(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    page: PageRequest<ActorId>,
) -> Result<Vec<WorkspaceOwner>, Error> {
    let rows = sqlx::query!(
        "SELECT o.user_id, u.display_name, o.created_at FROM workspace_owners o
         JOIN users u ON u.id = o.user_id
         WHERE o.workspace_id = $1 AND ($2::uuid IS NULL OR o.user_id > $2)
         ORDER BY o.user_id
         LIMIT $3",
        workspace.to_db(),
        page.after.map(ActorId::to_db),
        page.size.get() as i64 + 1,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| WorkspaceOwner {
            user_id: ActorId::from_db(row.user_id),
            display_name: row.display_name,
            since: row.created_at,
        })
        .collect())
}

/// Locks the workspaces that a user owns, in id order, then returns those where the user is the
/// only owner. The lock is the lock of `RemoveOwner`, so a removal at the same time waits.
pub(crate) async fn lock_sole_owned(
    tx: &mut Tx<'_>,
    user: ActorId,
) -> Result<Vec<WorkspaceId>, Error> {
    // Lock each workspace of the user.
    let owned = sqlx::query_scalar!(
        "SELECT w.id FROM workspaces w
         JOIN workspace_owners o ON o.workspace_id = w.id
         WHERE o.user_id = $1
         ORDER BY w.id
         FOR UPDATE OF w",
        user.to_db()
    )
    .fetch_all(&mut **tx)
    .await?;

    // Keep the workspaces where the user is still the one owner, after the lock.
    let sole = sqlx::query_scalar!(
        r#"SELECT workspace_id AS "id!" FROM workspace_owners
           WHERE workspace_id = ANY($1)
           GROUP BY workspace_id
           HAVING count(*) = 1 AND bool_and(user_id = $2)
           ORDER BY workspace_id"#,
        &owned,
        user.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(sole.into_iter().map(WorkspaceId::from_db).collect())
}

/// Returns `true` if a workspace holds a project or an agent.
pub(crate) async fn has_content(tx: &mut Tx<'_>, workspace: WorkspaceId) -> Result<bool, Error> {
    let found = sqlx::query_scalar!(
        r#"SELECT (EXISTS (SELECT 1 FROM projects WHERE workspace_id = $1)
                   OR EXISTS (SELECT 1 FROM agents WHERE workspace_id = $1)) AS "found!""#,
        workspace.to_db()
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(found)
}

/// Deletes workspaces, with their owners and their invitations.
pub(crate) async fn delete_all(tx: &mut Tx<'_>, ids: &[WorkspaceId]) -> Result<(), Error> {
    let ids: Vec<_> = ids.iter().map(|id| id.to_db()).collect();
    sqlx::query!("DELETE FROM workspaces WHERE id = ANY($1)", &ids)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Takes a user out of the owners of each workspace.
pub(crate) async fn leave_all(tx: &mut Tx<'_>, user: ActorId) -> Result<(), Error> {
    sqlx::query!(
        "DELETE FROM workspace_owners WHERE user_id = $1",
        user.to_db()
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
