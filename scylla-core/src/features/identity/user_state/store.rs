//! The SQL of the `user_state` table. Each function runs in the transaction of the caller.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::user_state::{StateKey, UserStateEntry};
use scylla_extension::Tx;
use serde_json::Value;
use time::OffsetDateTime;

/// What the entries of a user would use after a write of one entry.
pub(super) struct Usage {
    /// The number of entries.
    pub(super) entries: i64,
    /// The bytes of the keys and of the values.
    pub(super) bytes: i64,
}

/// Locks the row of a user. Returns `false` if no user has this id.
pub(super) async fn lock_user(tx: &mut Tx<'_>, user: ActorId) -> Result<bool, sqlx::Error> {
    let row = sqlx::query_scalar!(
        "SELECT id FROM users WHERE id = $1 FOR NO KEY UPDATE",
        user.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.is_some())
}

/// Returns each entry of a user, in key order.
pub(super) async fn list(
    tx: &mut Tx<'_>,
    user: ActorId,
) -> Result<Vec<UserStateEntry>, sqlx::Error> {
    let rows = sqlx::query_as!(
        Row,
        "SELECT key, value, version, updated_at FROM user_state WHERE user_id = $1 ORDER BY key",
        user.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(Row::into_entry).collect())
}

/// Returns the version of an entry, or `None` if the key has no entry.
pub(super) async fn version(
    tx: &mut Tx<'_>,
    user: ActorId,
    key: &StateKey,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT version FROM user_state WHERE user_id = $1 AND key = $2",
        user.to_db(),
        key.as_str(),
    )
    .fetch_optional(&mut **tx)
    .await
}

/// Returns what the entries of a user would use if this entry had this value.
pub(super) async fn usage_after(
    tx: &mut Tx<'_>,
    user: ActorId,
    key: &StateKey,
    value: &Value,
) -> Result<Usage, sqlx::Error> {
    sqlx::query_as!(
        Usage,
        r#"SELECT count(*) + 1 AS "entries!",
                  coalesce(sum(octet_length(key) + octet_length(value::text)), 0)::bigint
                      + octet_length($2::text) + octet_length($3::jsonb::text) AS "bytes!"
           FROM user_state
           WHERE user_id = $1 AND key <> $2::text"#,
        user.to_db(),
        key.as_str(),
        value,
    )
    .fetch_one(&mut **tx)
    .await
}

/// Writes an entry: version 1 for a new key, and one more for a key that has an entry.
pub(super) async fn upsert(
    tx: &mut Tx<'_>,
    user: ActorId,
    key: &StateKey,
    value: &Value,
) -> Result<UserStateEntry, sqlx::Error> {
    let row = sqlx::query_as!(
        Row,
        "INSERT INTO user_state (user_id, key, value) VALUES ($1, $2, $3)
         ON CONFLICT (user_id, key) DO UPDATE
         SET value = excluded.value, version = user_state.version + 1, updated_at = now()
         RETURNING key, value, version, updated_at",
        user.to_db(),
        key.as_str(),
        value,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.into_entry())
}

/// A row of the `user_state` table.
struct Row {
    /// The key.
    key: String,
    /// The value.
    value: Value,
    /// The version.
    version: i64,
    /// The time of the last write.
    updated_at: OffsetDateTime,
}

impl Row {
    /// Makes the entry of a row.
    fn into_entry(self) -> UserStateEntry {
        UserStateEntry {
            key: self.key,
            value_json: self.value.to_string(),
            version: self.version,
            updated_at: self.updated_at,
        }
    }
}
