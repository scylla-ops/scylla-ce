//! The SQL of the `instance_admins` table. Each function runs in the transaction of the caller.

use scylla_domain::identity::actors::ActorId;
use scylla_extension::Tx;

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
