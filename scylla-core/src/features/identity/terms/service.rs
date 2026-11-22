//! The logic of the terms of service: the check of a sign-up, the terms of a user, the
//! acceptance of the current version, and the proof of a deleted account.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::terms::{Acceptance, AcceptanceMethod, TermsState};
use scylla_domain::platform::legal::DocumentKind;
use scylla_extension::{Rejection, Tx};
use sqlx::PgPool;

use super::store;
use crate::features::platform::legal;
use crate::infra::secret;
use crate::Error;

/// Returns the version that each user must accept, or `None` when the terms are off or need no
/// acceptance.
async fn required(tx: &mut Tx<'_>) -> Result<Option<String>, Error> {
    let terms = legal::version(tx, DocumentKind::Terms).await?;
    Ok(terms
        .filter(|terms| terms.acceptance_required)
        .map(|terms| terms.version))
}

/// Checks the version of the terms that a sign-up accepts. Returns the version to keep with the
/// new user, or `None` when the terms need no acceptance. Another version gives
/// `TERMS_NOT_ACCEPTED`.
pub(crate) async fn check_sign_up(
    pool: &PgPool,
    accepted: Option<&str>,
) -> Result<Option<String>, Error> {
    let mut tx = pool.begin().await?;
    let required = required(&mut tx).await?;
    tx.commit().await?;
    match required {
        Some(version) if accepted != Some(version.as_str()) => Err(not_accepted(&version)),
        required => Ok(required),
    }
}

/// Keeps the acceptance of the sign-up form, in the transaction of the new user.
pub(crate) async fn record_sign_up(
    tx: &mut Tx<'_>,
    user: ActorId,
    version: &str,
) -> Result<(), Error> {
    store::insert(tx, user, version, AcceptanceMethod::SignUpForm).await?;
    Ok(())
}

/// Keeps the acceptances of a user as the proof, in the transaction of the deletion of its
/// account: the account id, the SHA-256 hash of its primary email in lowercase, the version, the
/// method and the date. The retention task deletes them some time after the deletion.
pub(crate) async fn keep_as_proof(
    tx: &mut Tx<'_>,
    user: ActorId,
    email: Option<&str>,
) -> Result<(), Error> {
    let email_hash = email.map(|email| secret::hash(&email.to_lowercase()));
    store::keep_as_proof(tx, user, email_hash.as_deref()).await?;
    Ok(())
}

/// Returns the terms of service of a user: the need of an acceptance and the last acceptance.
pub(crate) async fn state(pool: &PgPool, user: ActorId) -> Result<TermsState, Error> {
    let mut tx = pool.begin().await?;
    let accepted = store::last(&mut tx, user).await?;
    let acceptance_required = match required(&mut tx).await? {
        Some(version) => store::find(&mut tx, user, &version).await?.is_none(),
        None => false,
    };
    tx.commit().await?;
    Ok(TermsState {
        acceptance_required,
        accepted,
    })
}

/// Records that a user accepts the current version of the terms, and returns the acceptance.
/// Another version, or terms that are off, give `TERMS_NOT_ACCEPTED`.
pub(crate) async fn accept(
    pool: &PgPool,
    user: ActorId,
    version: &str,
) -> Result<Acceptance, Error> {
    // Refuse a version that is not the version of the terms that are on.
    let mut tx = pool.begin().await?;
    let current = legal::version(&mut tx, DocumentKind::Terms).await?;
    match current {
        Some(terms) if terms.version == version => {}
        current => {
            return Err(not_accepted(
                &current.map(|terms| terms.version).unwrap_or_default(),
            ))
        }
    }

    // Keep the acceptance: a first acceptance, or an acceptance after an earlier version.
    let method = match store::last(&mut tx, user).await? {
        Some(_) => AcceptanceMethod::ReAcceptance,
        None => AcceptanceMethod::AcceptanceStep,
    };
    store::insert(&mut tx, user, version, method).await?;
    let acceptance = store::find(&mut tx, user, version)
        .await?
        .ok_or(Error::NotFound("terms acceptance"))?;
    tx.commit().await?;
    Ok(acceptance)
}

/// The refusal of a request that does not accept the current version of the terms. The detail
/// `version` names the current version, empty when the terms are off.
fn not_accepted(current: &str) -> Error {
    let message = "the request does not accept the current version of the terms";
    Error::Rejected(Rejection::new("TERMS_NOT_ACCEPTED", message).detail("version", current))
}
