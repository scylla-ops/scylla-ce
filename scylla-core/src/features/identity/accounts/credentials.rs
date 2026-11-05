//! The check of a password: the same answer for each refusal, the count of the failures, and the
//! lockout of an account.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::users::{Identifier, UserStatus};
use scylla_extension::AccessError;
use time::OffsetDateTime;

use super::service::refused;
use super::{password, store, AccountContext};
use crate::Error;

/// Checks an email or a handle and a password, and returns the user. Each refusal has the same
/// answer: an identifier that is not valid, an unknown one, and a wrong password. A locked account
/// is not checked. `label` names the attempt in the log, and it is never the plain identifier.
pub(crate) async fn verify_password(
    ctx: &AccountContext,
    identifier: String,
    label: &str,
    password: String,
) -> Result<ActorId, Error> {
    // Read the credentials, and check the password even for an unknown identifier.
    let credentials = match Identifier::parse(identifier) {
        Ok(identifier) => {
            let mut tx = ctx.pool.begin().await?;
            let credentials = store::credentials(&mut tx, &identifier).await?;
            tx.commit().await?;
            credentials
        }
        Err(_) => None,
    };
    let locked = credentials
        .as_ref()
        .and_then(|credentials| credentials.locked_until)
        .is_some_and(|until| until > OffsetDateTime::now_utc());
    let hash = credentials
        .as_ref()
        .and_then(|credentials| credentials.hash.clone());
    let matches = password::verify(password, hash.filter(|_| !locked)).await;

    // Count a failure, then refuse.
    let credentials = match credentials {
        Some(credentials) if matches => credentials,
        Some(credentials) => {
            let mut tx = ctx.pool.begin().await?;
            let now_locked = store::record_failure(&mut tx, credentials.user).await?;
            tx.commit().await?;
            failed(label, if locked { "locked" } else { "password" });
            if now_locked {
                tracing::info!(event = "account.locked", user = %credentials.user);
            }
            return Err(invalid());
        }
        None => {
            failed(label, "unknown");
            return Err(invalid());
        }
    };

    // Refuse an account that is not active.
    match credentials.status {
        UserStatus::Active => Ok(credentials.user),
        UserStatus::Pending => {
            failed(label, "not_verified");
            Err(refused("EMAIL_NOT_VERIFIED", "verify your email first"))
        }
        UserStatus::Disabled => {
            failed(label, "disabled");
            Err(invalid())
        }
    }
}

/// Logs the event `account.signin_failed`, with the label of the attempt and the reason.
fn failed(identifier: &str, reason: &'static str) {
    tracing::info!(event = "account.signin_failed", identifier, reason);
}

/// The answer of each refused password check.
fn invalid() -> Error {
    AccessError::Unauthenticated("invalid identifier or password".to_owned()).into()
}
