//! The SQL of the `users` table. Each function runs in the transaction of the caller.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::admins::ListUsers;
use scylla_domain::identity::users::{ProfileChange, User, UserStatus};
use scylla_extension::{Rejection, Tx};

use time::OffsetDateTime;
use uuid::Uuid;

use crate::Error;

/// The unique constraint of the handles.
const HANDLE_KEY: &str = "users_handle_key";

/// A user row, with its primary email and its admin flag.
struct UserRow {
    /// The id.
    id: Uuid,
    /// The display name.
    display_name: String,
    /// The handle.
    handle: Option<String>,
    /// The primary email.
    email: Option<String>,
    /// The state of the account.
    status: String,
    /// `true` for an instance admin.
    instance_admin: bool,
    /// The creation time.
    created_at: OffsetDateTime,
}

impl From<UserRow> for User {
    fn from(row: UserRow) -> Self {
        Self {
            id: ActorId::from_db(row.id),
            display_name: row.display_name,
            handle: row.handle,
            email: row.email,
            status: UserStatus::from_name(&row.status),
            instance_admin: row.instance_admin,
            created_at: row.created_at,
        }
    }
}

/// Inserts the row of a user, for an actor of kind `user`. A used handle gives `HANDLE_IN_USE`.
pub(crate) async fn insert_user(
    tx: &mut Tx<'_>,
    id: ActorId,
    display_name: &str,
    handle: Option<&str>,
    status: UserStatus,
) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO users (id, display_name, handle, status) VALUES ($1, $2, $3, $4)",
        id.to_db(),
        display_name,
        handle,
        status.as_str(),
    )
    .execute(&mut **tx)
    .await
    .map_err(handle_error)?;
    Ok(())
}

/// Returns a user with its primary email, or `None` if no user has this id.
pub(crate) async fn find_user(tx: &mut Tx<'_>, id: ActorId) -> Result<Option<User>, Error> {
    let row = sqlx::query_as!(
        UserRow,
        r#"SELECT u.id, u.display_name, u.handle, e.email AS "email?", u.status,
                  EXISTS (SELECT 1 FROM instance_admins a WHERE a.user_id = u.id)
                      AS "instance_admin!",
                  u.created_at
           FROM users u
           LEFT JOIN user_emails e ON e.user_id = u.id AND e.is_primary
           WHERE u.id = $1"#,
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(User::from))
}

/// Returns the users whose email, handle or display name contains the query, after the page
/// id, in id order, with one extra row.
pub(crate) async fn list_users(tx: &mut Tx<'_>, query: &ListUsers) -> Result<Vec<User>, Error> {
    let rows = sqlx::query_as!(
        UserRow,
        r#"SELECT u.id, u.display_name, u.handle, e.email AS "email?", u.status,
                  EXISTS (SELECT 1 FROM instance_admins a WHERE a.user_id = u.id)
                      AS "instance_admin!",
                  u.created_at
           FROM users u
           LEFT JOIN user_emails e ON e.user_id = u.id AND e.is_primary
           WHERE ($1 = ''
                  OR strpos(lower(u.display_name), lower($1)) > 0
                  OR strpos(coalesce(u.handle, ''), lower($1)) > 0
                  OR strpos(coalesce(e.email, ''), lower($1)) > 0)
             AND (NOT $4
                  OR EXISTS (SELECT 1 FROM instance_admins a WHERE a.user_id = u.id)
                  OR EXISTS (SELECT 1 FROM role_grants g
                             WHERE g.actor_id = u.id AND g.scope_kind = 'platform'))
             AND ($2::uuid IS NULL OR u.id > $2)
           ORDER BY u.id
           LIMIT $3"#,
        query.query.trim(),
        query.page.after.map(ActorId::to_db),
        query.page.size.get() as i64 + 1,
        query.with_instance_role,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(User::from).collect())
}

/// Locks the row of a user until the end of the transaction. Returns `false` if no user has
/// this id.
pub(crate) async fn lock_user(tx: &mut Tx<'_>, id: ActorId) -> Result<bool, Error> {
    let row = sqlx::query_scalar!("SELECT id FROM users WHERE id = $1 FOR UPDATE", id.to_db())
        .fetch_optional(&mut **tx)
        .await?;
    Ok(row.is_some())
}

/// Changes the display name or the handle of a user. A used handle gives `HANDLE_IN_USE`.
pub(crate) async fn update_profile(
    tx: &mut Tx<'_>,
    id: ActorId,
    change: &ProfileChange,
) -> Result<(), Error> {
    let handle = change.handle.as_ref().map(|handle| handle.as_ref());
    sqlx::query!(
        "UPDATE users
         SET display_name = coalesce($2, display_name),
             handle = CASE WHEN $3 THEN $4 ELSE handle END
         WHERE id = $1",
        id.to_db(),
        change.display_name.as_ref().map(|name| name.as_str()),
        handle.is_some(),
        handle.flatten().map(|handle| handle.as_str()),
    )
    .execute(&mut **tx)
    .await
    .map_err(handle_error)?;
    Ok(())
}

/// Sets the state of a user.
pub(crate) async fn set_status(
    tx: &mut Tx<'_>,
    id: ActorId,
    status: UserStatus,
) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE users SET status = $2 WHERE id = $1",
        id.to_db(),
        status.as_str(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Records a sign-in: no failure and no lock, and the time of the sign-in.
pub(crate) async fn record_sign_in(tx: &mut Tx<'_>, user: ActorId) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE passwords SET failed_attempts = 0, locked_until = NULL WHERE user_id = $1",
        user.to_db()
    )
    .execute(&mut **tx)
    .await?;
    sqlx::query!(
        "UPDATE users SET last_sign_in_at = now() WHERE id = $1",
        user.to_db()
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Deletes the row of a user: its emails, password, sessions, identities, one-time tokens and
/// admin row go with it. Its actor stays, so each `created_by` stays valid.
pub(crate) async fn delete_user(tx: &mut Tx<'_>, id: ActorId) -> Result<(), Error> {
    sqlx::query!("DELETE FROM users WHERE id = $1", id.to_db())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Maps the violation of the unique handle to `HANDLE_IN_USE`.
fn handle_error(error: sqlx::Error) -> Error {
    match error {
        sqlx::Error::Database(db) if db.constraint() == Some(HANDLE_KEY) => Error::Rejected(
            Rejection::new("HANDLE_IN_USE", "another account uses this handle"),
        ),
        error => error.into(),
    }
}
