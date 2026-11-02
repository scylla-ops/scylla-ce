//! The one-time tokens of the mail links: a check of an email and a password reset. The
//! database keeps their hashes only.

use scylla_domain::identity::actors::ActorId;
use scylla_extension::Tx;
use time::{Duration, OffsetDateTime};

use crate::infra::secret;
use crate::Error;

/// What a one-time token is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    /// The check of the email of a new user: valid one day.
    VerifyEmail,
    /// A password reset: valid one hour.
    ResetPassword,
}

impl Purpose {
    /// Returns the name in the database.
    fn as_str(self) -> &'static str {
        match self {
            Self::VerifyEmail => "verify_email",
            Self::ResetPassword => "reset_password",
        }
    }

    /// Returns the lifetime of a token.
    fn lifetime(self) -> Duration {
        match self {
            Self::VerifyEmail => Duration::days(1),
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

/// Uses a one-time token. Returns its owner, or `None` for an unknown, used or expired token.
pub(crate) async fn redeem(
    tx: &mut Tx<'_>,
    purpose: Purpose,
    token: &str,
) -> Result<Option<Redeemed>, Error> {
    let row = sqlx::query!(
        "UPDATE one_time_tokens SET used_at = now()
         WHERE hash = $1 AND purpose = $2 AND used_at IS NULL AND expires_at > now()
         RETURNING user_id, email",
        secret::hash(token),
        purpose.as_str(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| Redeemed {
        user: ActorId::from_db(row.user_id),
        email: row.email,
    }))
}
