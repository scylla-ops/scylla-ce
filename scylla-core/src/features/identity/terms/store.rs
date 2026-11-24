//! The SQL of the `terms_acceptances` table: one row for each version that a user accepted. The
//! rows of a deleted account stay as the proof, with the time of the deletion and the hash of its
//! email.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::terms::{Acceptance, AcceptanceMethod};
use scylla_domain::platform::legal::DocumentKind;
use scylla_extension::Tx;

/// The kind of the terms of service.
const KIND: DocumentKind = DocumentKind::Terms;

/// Adds the acceptance of a version. A second acceptance of the same version keeps the first.
pub(super) async fn insert(
    tx: &mut Tx<'_>,
    user: ActorId,
    version: &str,
    method: AcceptanceMethod,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO terms_acceptances (user_id, kind, version, method)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id, kind, version) DO NOTHING",
        user.to_db(),
        KIND.as_str(),
        version,
        method.as_str(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the acceptance of a version by a user, if any.
pub(super) async fn find(
    tx: &mut Tx<'_>,
    user: ActorId,
    version: &str,
) -> Result<Option<Acceptance>, sqlx::Error> {
    sqlx::query_as!(
        Acceptance,
        "SELECT version, accepted_at FROM terms_acceptances
         WHERE user_id = $1 AND kind = $2 AND version = $3",
        user.to_db(),
        KIND.as_str(),
        version,
    )
    .fetch_optional(&mut **tx)
    .await
}

/// Returns the last acceptance of a user, if any.
pub(super) async fn last(
    tx: &mut Tx<'_>,
    user: ActorId,
) -> Result<Option<Acceptance>, sqlx::Error> {
    sqlx::query_as!(
        Acceptance,
        "SELECT version, accepted_at FROM terms_acceptances
         WHERE user_id = $1 AND kind = $2
         ORDER BY accepted_at DESC, version DESC
         LIMIT 1",
        user.to_db(),
        KIND.as_str(),
    )
    .fetch_optional(&mut **tx)
    .await
}

/// Marks the acceptances of a user as the proof of a deleted account, with the time of the
/// deletion and the hash of the email of the account.
pub(super) async fn keep_as_proof(
    tx: &mut Tx<'_>,
    user: ActorId,
    email_hash: Option<&[u8]>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE terms_acceptances SET account_deleted_at = now(), email_hash = $2
         WHERE user_id = $1",
        user.to_db(),
        email_hash,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
