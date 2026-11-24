//! The SQL of the `user_emails` table. Each function runs in the transaction of the caller.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::users::{Email, UserStatus};
use scylla_extension::{Rejection, Tx};

use crate::Error;

/// The primary key of the emails: one user for each email.
const EMAIL_KEY: &str = "user_emails_pkey";

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

/// The account that has an email, as a sign-in with a provider reads it.
pub(crate) struct EmailAccount {
    /// The user.
    pub(crate) user: ActorId,
    /// The state of the account.
    pub(crate) status: UserStatus,
    /// `true` if the account has a password.
    pub(crate) has_password: bool,
}

/// Returns the account that has an email, or `None`.
pub(crate) async fn account_of_email(
    tx: &mut Tx<'_>,
    email: &Email,
) -> Result<Option<EmailAccount>, Error> {
    let row = sqlx::query!(
        r#"SELECT e.user_id, u.status,
                  EXISTS (SELECT 1 FROM passwords p WHERE p.user_id = e.user_id)
                      AS "has_password!"
           FROM user_emails e JOIN users u ON u.id = e.user_id
           WHERE e.email = $1"#,
        email.as_str()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| EmailAccount {
        user: ActorId::from_db(row.user_id),
        status: UserStatus::from_name(&row.status),
        has_password: row.has_password,
    }))
}

/// Replaces the emails of a user with one primary email, verified. An email of another user
/// gives `EMAIL_IN_USE`.
pub(crate) async fn replace_email(
    tx: &mut Tx<'_>,
    user: ActorId,
    email: &Email,
) -> Result<(), Error> {
    sqlx::query!("DELETE FROM user_emails WHERE user_id = $1", user.to_db())
        .execute(&mut **tx)
        .await?;
    sqlx::query!(
        "INSERT INTO user_emails (email, user_id, verified_at) VALUES ($1, $2, now())",
        email.as_str(),
        user.to_db(),
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(db) if db.constraint() == Some(EMAIL_KEY) => email_in_use(),
        error => error.into(),
    })?;
    Ok(())
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

/// The refusal of an email that another account uses.
pub(crate) fn email_in_use() -> Error {
    Error::Rejected(Rejection::new("EMAIL_IN_USE", "an account uses this email"))
}
