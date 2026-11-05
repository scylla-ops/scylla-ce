//! The SQL of the `instance_admins` table and of the facts of the console of the users. Each
//! function runs in the transaction of the caller.

use std::collections::HashMap;

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::admins::InstanceRole;
use scylla_domain::tenancy::roles::{RoleId, WORKSPACE_CREATOR, WORKSPACE_CREATOR_ROW};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;
use time::OffsetDateTime;

use crate::Error;

/// The foreign key from an admin to its user.
const USER_FOREIGN_KEY: &str = "instance_admins_user_id_fkey";

/// An instance admin, as the rule of the last admin reads it.
pub(super) struct Admin {
    /// The user.
    pub(super) user: ActorId,
    /// `true` if the account of the user is active.
    pub(super) active: bool,
}

/// Locks the rows of the instance admins until the end of the transaction, then returns them
/// with the states of their accounts, read after the lock.
pub(super) async fn lock_all(tx: &mut Tx<'_>) -> Result<Vec<Admin>, Error> {
    // Lock the admins.
    let locked =
        sqlx::query_scalar!("SELECT user_id FROM instance_admins ORDER BY user_id FOR UPDATE")
            .fetch_all(&mut **tx)
            .await?;

    // Read the states after the lock, in a new statement.
    let rows = sqlx::query!(
        r#"SELECT a.user_id, u.status = 'active' AS "active!"
           FROM instance_admins a JOIN users u ON u.id = a.user_id
           WHERE a.user_id = ANY($1)
           ORDER BY a.user_id"#,
        &locked,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Admin {
            user: ActorId::from_db(row.user_id),
            active: row.active,
        })
        .collect())
}

/// Makes a user an instance admin. An admin stays an admin. An unknown user gives `NotFound`.
pub(super) async fn grant(tx: &mut Tx<'_>, user: ActorId) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO instance_admins (user_id) VALUES ($1) ON CONFLICT DO NOTHING",
        user.to_db()
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

/// Takes the instance admin right from a user. A user that is not an admin changes nothing.
pub(super) async fn revoke(tx: &mut Tx<'_>, user: ActorId) -> Result<(), Error> {
    sqlx::query!(
        "DELETE FROM instance_admins WHERE user_id = $1",
        user.to_db()
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the instance roles that are rows of each user, in name order.
pub(super) async fn platform_roles(
    tx: &mut Tx<'_>,
    users: &[ActorId],
) -> Result<Vec<(ActorId, InstanceRole)>, Error> {
    let ids: Vec<_> = users.iter().map(|user| user.to_db()).collect();
    let rows = sqlx::query!(
        "SELECT g.actor_id, r.id, r.name, r.builtin
         FROM role_grants g JOIN roles r ON r.id = g.role_id
         WHERE g.actor_id = ANY($1) AND g.scope_kind = 'platform'
         ORDER BY r.name",
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let id = RoleId::from_db(row.id);
            let key = if id == WORKSPACE_CREATOR_ROW {
                WORKSPACE_CREATOR.to_owned()
            } else {
                id.to_string()
            };
            let role = InstanceRole {
                key,
                name: row.name,
                builtin: row.builtin,
            };
            (ActorId::from_db(row.actor_id), role)
        })
        .collect())
}

/// Returns the workspaces that each user owns, with their names.
pub(super) async fn owned_workspaces(
    tx: &mut Tx<'_>,
    users: &[ActorId],
) -> Result<Vec<(ActorId, WorkspaceId, String)>, Error> {
    let ids: Vec<_> = users.iter().map(|user| user.to_db()).collect();
    let rows = sqlx::query!(
        "SELECT o.user_id, w.id, w.name
         FROM workspace_owners o JOIN workspaces w ON w.id = o.workspace_id
         WHERE o.user_id = ANY($1)",
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let workspace = WorkspaceId::from_db(row.id);
            (ActorId::from_db(row.user_id), workspace, row.name)
        })
        .collect())
}

/// Returns the names of workspaces, by id.
pub(super) async fn workspace_names(
    tx: &mut Tx<'_>,
    workspaces: &[WorkspaceId],
) -> Result<HashMap<WorkspaceId, String>, Error> {
    let ids: Vec<_> = workspaces.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query!("SELECT id, name FROM workspaces WHERE id = ANY($1)", &ids)
        .fetch_all(&mut **tx)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| (WorkspaceId::from_db(row.id), row.name))
        .collect())
}

/// Returns the time of the last sign-in of each user that signed in.
pub(super) async fn last_sign_ins(
    tx: &mut Tx<'_>,
    users: &[ActorId],
) -> Result<HashMap<ActorId, OffsetDateTime>, Error> {
    let ids: Vec<_> = users.iter().map(|user| user.to_db()).collect();
    let rows = sqlx::query!(
        r#"SELECT user_id, max(created_at) AS "at!" FROM sessions
           WHERE user_id = ANY($1) GROUP BY user_id"#,
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (ActorId::from_db(row.user_id), row.at))
        .collect())
}

/// Returns the number of users that a query keeps, then those of them with an instance role.
pub(super) async fn totals(tx: &mut Tx<'_>, query: &str) -> Result<(u32, u32), Error> {
    let row = sqlx::query!(
        r#"SELECT count(*) AS "total!",
                  count(*) FILTER (WHERE
                      EXISTS (SELECT 1 FROM instance_admins a WHERE a.user_id = u.id)
                      OR EXISTS (SELECT 1 FROM role_grants g
                                 WHERE g.actor_id = u.id AND g.scope_kind = 'platform'))
                      AS "with_role!"
           FROM users u
           LEFT JOIN user_emails e ON e.user_id = u.id AND e.is_primary
           WHERE ($1 = ''
                  OR strpos(lower(u.display_name), lower($1)) > 0
                  OR strpos(coalesce(u.handle, ''), lower($1)) > 0
                  OR strpos(coalesce(e.email, ''), lower($1)) > 0)"#,
        query,
    )
    .fetch_one(&mut **tx)
    .await?;
    let count = |value: i64| u32::try_from(value).unwrap_or(u32::MAX);
    Ok((count(row.total), count(row.with_role)))
}
