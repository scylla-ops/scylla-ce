//! The one-time tokens of the mail links: a check of an email, a password reset, and a change
//! of email. The database keeps their hashes only.

use std::time::Duration as StdDuration;

use scylla_domain::identity::actors::ActorId;
use scylla_extension::Tx;
use time::{Duration, OffsetDateTime};

use crate::infra::secret;
use crate::Error;

/// The shortest time between two links of one purpose to one account.
pub(super) const COOLDOWN: StdDuration = StdDuration::from_secs(60);

/// What a one-time token is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    /// The check of the email of a new user: valid one day.
    VerifyEmail,
    /// A password reset: valid one hour.
    ResetPassword,
    /// A change of the email of a user, to the email of the token: valid one day.
    ChangeEmail,
}

impl Purpose {
    /// Returns the name in the database.
    fn as_str(self) -> &'static str {
        match self {
            Self::VerifyEmail => "verify_email",
            Self::ResetPassword => "reset_password",
            Self::ChangeEmail => "change_email",
        }
    }

    /// Returns the lifetime of a token.
    fn lifetime(self) -> Duration {
        match self {
            Self::VerifyEmail | Self::ChangeEmail => Duration::days(1),
            Self::ResetPassword => Duration::hours(1),
        }
    }
}

/// The owner of a one-time token that was valid.
pub(crate) struct Redeemed {
    /// The user of the token.
    pub(crate) user: ActorId,
    /// The email that the link went to.
    pub(crate) email: String,
}

/// Makes a one-time token for a user and an email, and returns it.
pub(crate) async fn create(
    tx: &mut Tx<'_>,
    purpose: Purpose,
    user: ActorId,
    email: &str,
) -> Result<String, Error> {
    let token = secret::token();
    sqlx::query!(
        "INSERT INTO one_time_tokens (hash, purpose, user_id, email, expires_at)
         VALUES ($1, $2, $3, $4, $5)",
        secret::hash(&token),
        purpose.as_str(),
        user.to_db(),
        email,
        OffsetDateTime::now_utc() + purpose.lifetime(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(token)
}

/// Uses a one-time token, of one user if one is given. Returns its owner, or `None` for an
/// unknown, used or expired token, a token of another user, or a token to an email that is not
/// an email of the user any more. The token of a change of email goes to a new email.
pub(crate) async fn redeem(
    tx: &mut Tx<'_>,
    purpose: Purpose,
    token: &str,
    user: Option<ActorId>,
) -> Result<Option<Redeemed>, Error> {
    let row = sqlx::query!(
        "UPDATE one_time_tokens t SET used_at = now()
         WHERE t.hash = $1 AND t.purpose = $2 AND t.used_at IS NULL AND t.expires_at > now()
           AND ($3::uuid IS NULL OR t.user_id = $3)
           AND ($4 OR EXISTS (
               SELECT 1 FROM user_emails e WHERE e.user_id = t.user_id AND e.email = t.email))
         RETURNING t.user_id, t.email",
        secret::hash(token),
        purpose.as_str(),
        user.map(ActorId::to_db),
        purpose == Purpose::ChangeEmail,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| Redeemed {
        user: ActorId::from_db(row.user_id),
        email: row.email,
    }))
}

/// Returns `true` if a user got a token of this purpose less than `COOLDOWN` ago.
pub(super) async fn is_recent(
    tx: &mut Tx<'_>,
    purpose: Purpose,
    user: ActorId,
) -> Result<bool, Error> {
    let recent = sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM one_time_tokens
               WHERE user_id = $1 AND purpose = $2
                 AND created_at > now() - make_interval(secs => $3)
           ) AS "recent!""#,
        user.to_db(),
        purpose.as_str(),
        COOLDOWN.as_secs_f64(),
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(recent)
}

/// Returns `true` if an email got a token of this purpose less than `COOLDOWN` ago, for any
/// user. It waits for the other requests to the same email first, until the end of the
/// transaction.
pub(super) async fn is_recent_to(
    tx: &mut Tx<'_>,
    purpose: Purpose,
    email: &str,
) -> Result<bool, Error> {
    // Wait for the other requests to the same email.
    sqlx::query_scalar!(
        r#"SELECT true AS "locked!" FROM pg_advisory_xact_lock(hashtextextended($1, 0))"#,
        email,
    )
    .fetch_one(&mut **tx)
    .await?;

    // Look for a recent token to the email.
    let recent = sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM one_time_tokens
               WHERE email = $1 AND purpose = $2
                 AND created_at > now() - make_interval(secs => $3)
           ) AS "recent!""#,
        email,
        purpose.as_str(),
        COOLDOWN.as_secs_f64(),
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(recent)
}

/// Stops each unused token of this purpose of a user: a new link replaces the old ones.
pub(super) async fn cancel(tx: &mut Tx<'_>, purpose: Purpose, user: ActorId) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE one_time_tokens SET used_at = now()
         WHERE user_id = $1 AND purpose = $2 AND used_at IS NULL",
        user.to_db(),
        purpose.as_str(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Stops each unused token of a user, of each purpose: the links that went to an old email.
pub(super) async fn cancel_all(tx: &mut Tx<'_>, user: ActorId) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE one_time_tokens SET used_at = now() WHERE user_id = $1 AND used_at IS NULL",
        user.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
