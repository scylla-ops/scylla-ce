//! The SQL of the retention: the deletes of the rows that ended before a time. Each function runs
//! in the transaction of the caller.

use scylla_extension::Tx;
use time::OffsetDateTime;

use crate::Error;

/// Deletes the sessions that ended or that a person closed before a time.
pub(super) async fn purge_sessions(tx: &mut Tx<'_>, before: OffsetDateTime) -> Result<(), Error> {
    sqlx::query!(
        "DELETE FROM sessions WHERE least(revoked_at, expires_at) < $1",
        before
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Deletes the one-time tokens that were used or that ended before a time.
pub(super) async fn purge_one_time_tokens(
    tx: &mut Tx<'_>,
    before: OffsetDateTime,
) -> Result<(), Error> {
    sqlx::query!(
        "DELETE FROM one_time_tokens WHERE least(used_at, expires_at) < $1",
        before
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Deletes the invitations that were accepted or that ended before a time.
pub(super) async fn purge_invitations(
    tx: &mut Tx<'_>,
    before: OffsetDateTime,
) -> Result<(), Error> {
    sqlx::query!(
        "DELETE FROM invitations WHERE least(accepted_at, expires_at) < $1",
        before
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Deletes the proof of the acceptances of the terms of the accounts deleted before a time.
pub(super) async fn purge_terms_proofs(
    tx: &mut Tx<'_>,
    before: OffsetDateTime,
) -> Result<(), Error> {
    sqlx::query!(
        "DELETE FROM terms_acceptances WHERE account_deleted_at < $1",
        before
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
