//! The SQL of the limit of the workspaces that one user creates.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;

use crate::Error;

/// Locks the row of a user until the end of the transaction.
pub(crate) async fn lock_creator(tx: &mut Tx<'_>, user: ActorId) -> Result<(), Error> {
    sqlx::query_scalar!(
        "SELECT id FROM users WHERE id = $1 FOR UPDATE",
        user.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(())
}

/// Returns `true` if a user has no limit: an instance admin.
pub(crate) async fn has_no_limit(tx: &mut Tx<'_>, user: ActorId) -> Result<bool, Error> {
    let admin = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM instance_admins WHERE user_id = $1) AS "admin!""#,
        user.to_db(),
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(admin)
}

/// Returns the number of workspaces that count for the limit of a user.
pub(crate) async fn counted_for(tx: &mut Tx<'_>, user: ActorId) -> Result<i64, Error> {
    let count = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM workspaces WHERE counts_for = $1"#,
        user.to_db(),
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(count)
}

/// Makes a workspace count for the limit of a user.
pub(crate) async fn count_for(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    user: ActorId,
) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE workspaces SET counts_for = $2 WHERE id = $1",
        workspace.to_db(),
        user.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
