//! The SQL of the users, their emails and their passwords. Each function runs in the
//! transaction of the caller.

use scylla_domain::actors::ActorId;
use scylla_domain::users::{Email, User, UserStatus};
use scylla_extension::Tx;
use time::OffsetDateTime;

use crate::Error;

/// What a password sign-in reads about an email.
pub(super) struct Credentials {
    /// The user of the email.
    pub(super) user: ActorId,
    /// The state of the account.
    pub(super) status: UserStatus,
    /// The hash of the password, if the user has one.
    pub(super) hash: Option<String>,
    /// The end of the lock after too many failures, if there is one.
    pub(super) locked_until: Option<OffsetDateTime>,
}

/// Inserts the row of a user, for an actor of kind `user`.
pub(crate) async fn insert_user(
    tx: &mut Tx<'_>,
    id: ActorId,
    display_name: &str,
    status: UserStatus,
) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO users (id, display_name, status) VALUES ($1, $2, $3)",
        id.to_db(),
        display_name,
        status.as_str(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Inserts the primary email of a user.
pub(crate) async fn insert_email(
    tx: &mut Tx<'_>,
    user: ActorId,
    email: &Email,
    verified: bool,
) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO user_emails (email, user_id, verified_at)
         VALUES ($1, $2, CASE WHEN $3 THEN now() END)",
        email.as_str(),
        user.to_db(),
        verified,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the user of an email, or `None` if no user has it.
pub(crate) async fn user_of_email(
    tx: &mut Tx<'_>,
    email: &Email,
) -> Result<Option<ActorId>, Error> {
    let user = sqlx::query_scalar!(
        "SELECT user_id FROM user_emails WHERE email = $1",
        email.as_str()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(user.map(ActorId::from_db))
}

/// Returns what a password sign-in needs for an email, or `None` if no user has it.
pub(super) async fn credentials(
    tx: &mut Tx<'_>,
    email: &Email,
) -> Result<Option<Credentials>, Error> {
    let row = sqlx::query!(
        r#"SELECT e.user_id, u.status, p.hash AS "hash?", p.locked_until
           FROM user_emails e
           JOIN users u ON u.id = e.user_id
           LEFT JOIN passwords p ON p.user_id = e.user_id
           WHERE e.email = $1"#,
        email.as_str()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| Credentials {
        user: ActorId::from_db(row.user_id),
        status: UserStatus::from_name(&row.status),
        hash: row.hash,
        locked_until: row.locked_until,
    }))
}

/// Records a failed password: after 5 failures, each new one locks the account for longer, up
/// to one hour.
pub(super) async fn record_failure(tx: &mut Tx<'_>, user: ActorId) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE passwords
         SET failed_attempts = failed_attempts + 1,
             locked_until = CASE WHEN failed_attempts + 1 >= 5
                 THEN now() + least(interval '1 hour',
                                    interval '1 minute' * power(2, failed_attempts + 1 - 5))
                 END
         WHERE user_id = $1",
        user.to_db()
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

/// Sets the password of a user, and removes its lock.
pub(super) async fn set_password(tx: &mut Tx<'_>, user: ActorId, hash: &str) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO passwords (user_id, hash) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE
         SET hash = $2, failed_attempts = 0, locked_until = NULL, updated_at = now()",
        user.to_db(),
        hash,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the hash of the password of a user, if the user has one.
pub(crate) async fn password_hash(tx: &mut Tx<'_>, user: ActorId) -> Result<Option<String>, Error> {
    let hash = sqlx::query_scalar!(
        "SELECT hash FROM passwords WHERE user_id = $1",
        user.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(hash)
}

/// Marks an email as verified, and activates its user if it was pending.
pub(crate) async fn verify_email(tx: &mut Tx<'_>, user: ActorId, email: &str) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE user_emails SET verified_at = coalesce(verified_at, now())
         WHERE email = $1 AND user_id = $2",
        email,
        user.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    sqlx::query!(
        "UPDATE users SET status = 'active' WHERE id = $1 AND status = 'pending'",
        user.to_db()
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the verified emails of a user.
pub(crate) async fn verified_emails(tx: &mut Tx<'_>, user: ActorId) -> Result<Vec<String>, Error> {
    let emails = sqlx::query_scalar!(
        "SELECT email FROM user_emails WHERE user_id = $1 AND verified_at IS NOT NULL",
        user.to_db()
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(emails)
}

/// Returns a user with its primary email, or `None` if no user has this id.
pub(crate) async fn find_user(tx: &mut Tx<'_>, id: ActorId) -> Result<Option<User>, Error> {
    let row = sqlx::query!(
        r#"SELECT u.id, u.display_name, u.status, u.created_at, e.email AS "email?"
           FROM users u
           LEFT JOIN user_emails e ON e.user_id = u.id AND e.is_primary
           WHERE u.id = $1"#,
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| User {
        id: ActorId::from_db(row.id),
        display_name: row.display_name,
        email: row.email,
        status: UserStatus::from_name(&row.status),
        created_at: row.created_at,
    }))
}
