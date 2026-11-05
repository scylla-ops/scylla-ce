//! The SQL of the `workspace_owners` table.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;

use crate::Error;

/// The foreign key from an owner to its workspace.
const WORKSPACE_FOREIGN_KEY: &str = "workspace_owners_workspace_id_fkey";

/// The foreign key from an owner to its user.
const USER_FOREIGN_KEY: &str = "workspace_owners_user_id_fkey";

/// Makes a user an owner of a workspace. An owner stays an owner. An unknown user or an
/// unknown workspace gives `NotFound`.
pub(crate) async fn add_owner(
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
        sqlx::Error::Database(db) if db.constraint() == Some(WORKSPACE_FOREIGN_KEY) => {
            Error::NotFound("workspace")
        }
        error => error.into(),
    })?;
    Ok(())
}

/// Takes a user out of the owners of a workspace. A user that is not an owner changes nothing.
pub(crate) async fn remove_owner(
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
pub(crate) async fn count_owners(tx: &mut Tx<'_>, workspace: WorkspaceId) -> Result<i64, Error> {
    let count = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM workspace_owners WHERE workspace_id = $1"#,
        workspace.to_db()
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(count)
}

/// Locks the workspaces that a user owns, in id order, then returns those where the user is the
/// only owner. The lock is the lock of a removal, so a removal at the same time waits.
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
