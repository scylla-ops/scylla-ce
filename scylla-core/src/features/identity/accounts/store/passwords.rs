//! The SQL of the `passwords` table and of a password sign-in. Each function runs in the
//! transaction of the caller.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::users::{Identifier, UserStatus};
use scylla_extension::Tx;
use time::OffsetDateTime;

use crate::Error;

/// What a password sign-in reads about an email or a handle.
pub(crate) struct Credentials {
    /// The user of the email or the handle.
    pub(crate) user: ActorId,
    /// The state of the account.
    pub(crate) status: UserStatus,
    /// The hash of the password, if the user has one.
    pub(crate) hash: Option<String>,
    /// The end of the lock after too many failures, if there is one.
    pub(crate) locked_until: Option<OffsetDateTime>,
}

/// Returns what a password sign-in needs for an email or a handle, or `None` if no user has
/// it.
pub(crate) async fn credentials(
    tx: &mut Tx<'_>,
    identifier: &Identifier,
) -> Result<Option<Credentials>, Error> {
    let (email, handle) = match identifier {
        Identifier::Email(email) => (Some(email.as_str()), None),
        Identifier::Handle(handle) => (None, Some(handle.as_str())),
    };
    let row = sqlx::query!(
        r#"SELECT u.id, u.status, p.hash AS "hash?", p.locked_until AS "locked_until?"
           FROM users u
           LEFT JOIN passwords p ON p.user_id = u.id
           WHERE u.id = (SELECT user_id FROM user_emails WHERE email = $1)
              OR u.handle = $2"#,
        email,
        handle,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| Credentials {
        user: ActorId::from_db(row.id),
        status: UserStatus::from_name(&row.status),
        hash: row.hash,
        locked_until: row.locked_until,
    }))
}

/// Records a failed password: after 5 failures, each new one locks the account for longer, up
/// to one hour. A failure while the account is locked does not count.
pub(crate) async fn record_failure(tx: &mut Tx<'_>, user: ActorId) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE passwords
         SET failed_attempts = failed_attempts + 1,
             locked_until = CASE WHEN failed_attempts + 1 >= 5
                 THEN now() + least(interval '1 hour',
                                    interval '1 minute'
                                        * power(2, least(failed_attempts + 1 - 5, 6)))
                 END
         WHERE user_id = $1 AND (locked_until IS NULL OR locked_until <= now())",
        user.to_db()
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Sets the password of a user, and removes its lock.
pub(crate) async fn set_password(tx: &mut Tx<'_>, user: ActorId, hash: &str) -> Result<(), Error> {
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
