//! The logic of the state of a user: one transaction for each call.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::user_state::{StateKey, UserStateEntry, MAX_ENTRIES, MAX_TOTAL_BYTES};
use scylla_extension::Rejection;
use serde_json::Value;
use sqlx::PgPool;

use super::store;
use crate::Error;

/// Returns each entry of a user.
pub(super) async fn list(pool: &PgPool, user: ActorId) -> Result<Vec<UserStateEntry>, Error> {
    let mut tx = pool.begin().await?;
    let entries = store::list(&mut tx, user).await?;
    tx.commit().await?;
    Ok(entries)
}

/// Writes one entry. `expected` is the version that the client read, and 0 means no entry. A
/// write that puts the user above a limit gives `USER_STATE_FULL`, and another version gives
/// `ABORTED`.
pub(super) async fn put(
    pool: &PgPool,
    user: ActorId,
    key: &StateKey,
    value: &Value,
    expected: Option<i64>,
) -> Result<UserStateEntry, Error> {
    // Lock the user, so that two writes count the entries one after the other.
    let mut tx = pool.begin().await?;
    if !store::lock_user(&mut tx, user).await? {
        return Err(Error::NotFound("user"));
    }

    // Refuse a stale version.
    let current = store::version(&mut tx, user, key).await?;
    if expected.is_some_and(|version| version != current.unwrap_or(0)) {
        return Err(Error::Conflict("the entry changed: read it again"));
    }

    // Refuse a write above the limits of the user.
    let usage = store::usage_after(&mut tx, user, key, value).await?;
    if usage.entries > MAX_ENTRIES || usage.bytes > MAX_TOTAL_BYTES {
        let message = "the state of the user is full";
        return Err(Error::Rejected(Rejection::new("USER_STATE_FULL", message)));
    }

    // Write the entry.
    let entry = store::upsert(&mut tx, user, key, value).await?;
    tx.commit().await?;
    Ok(entry)
}
