//! The SQL of the `app_keys` table.

use scylla_domain::identity::apps::{AppId, AppKeyId, AppKeyInfo, KeyLabel};
use scylla_extension::Tx;
use time::OffsetDateTime;

/// A public key of an App, for the check of an assertion.
pub(crate) struct KeyRow {
    /// The App of the key.
    pub(crate) app_id: AppId,
    /// The Ed25519 public key, 32 bytes.
    pub(crate) public_key: Vec<u8>,
    /// `true` if the key is revoked.
    pub(crate) revoked: bool,
}

/// Inserts a public key of an App with an optional name, and returns it.
pub(crate) async fn insert(
    tx: &mut Tx<'_>,
    app: AppId,
    public_key: &[u8],
    label: Option<&KeyLabel>,
) -> Result<AppKeyInfo, sqlx::Error> {
    let row = sqlx::query_as!(
        Row,
        "INSERT INTO app_keys (id, app_id, public_key, label) VALUES ($1, $2, $3, $4)
         RETURNING id, public_key, label, created_at, revoked_at, last_used_at",
        AppKeyId::generate().to_db(),
        app.to_db(),
        public_key,
        label.map(KeyLabel::as_str),
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.into_info())
}

/// Returns a key, or `None` if it is unknown.
pub(crate) async fn find(tx: &mut Tx<'_>, id: AppKeyId) -> Result<Option<KeyRow>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT app_id, public_key, revoked_at IS NOT NULL AS "revoked!"
           FROM app_keys WHERE id = $1"#,
        id.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| KeyRow {
        app_id: AppId::from_db(row.app_id),
        public_key: row.public_key,
        revoked: row.revoked,
    }))
}

/// Returns the keys of an App, oldest first.
pub(crate) async fn list(tx: &mut Tx<'_>, app: AppId) -> Result<Vec<AppKeyInfo>, sqlx::Error> {
    let rows = sqlx::query_as!(
        Row,
        "SELECT id, public_key, label, created_at, revoked_at, last_used_at FROM app_keys
         WHERE app_id = $1 ORDER BY id",
        app.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(Row::into_info).collect())
}

/// Revokes a key of an App. Returns `false` if the App has no key with this id. A revoked key
/// keeps its first revocation time.
pub(crate) async fn revoke(tx: &mut Tx<'_>, app: AppId, id: AppKeyId) -> Result<bool, sqlx::Error> {
    let found: Option<OffsetDateTime> = sqlx::query_scalar!(
        r#"UPDATE app_keys SET revoked_at = coalesce(revoked_at, now())
           WHERE id = $1 AND app_id = $2 RETURNING revoked_at AS "revoked_at!""#,
        id.to_db(),
        app.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(found.is_some())
}

/// Gives a new name to a key of an App, and returns the key. Returns `None` if the App has no key
/// with this id.
pub(crate) async fn rename(
    tx: &mut Tx<'_>,
    app: AppId,
    id: AppKeyId,
    label: &KeyLabel,
) -> Result<Option<AppKeyInfo>, sqlx::Error> {
    let row = sqlx::query_as!(
        Row,
        "UPDATE app_keys SET label = $3 WHERE id = $1 AND app_id = $2
         RETURNING id, public_key, label, created_at, revoked_at, last_used_at",
        id.to_db(),
        app.to_db(),
        label.as_str(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(Row::into_info))
}

/// Sets the last use of a key to now.
pub(crate) async fn touch(tx: &mut Tx<'_>, id: AppKeyId) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE app_keys SET last_used_at = now() WHERE id = $1",
        id.to_db()
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// A row of the `app_keys` table.
struct Row {
    /// The id.
    id: uuid::Uuid,
    /// The Ed25519 public key.
    public_key: Vec<u8>,
    /// The name.
    label: Option<String>,
    /// The time of the insert.
    created_at: OffsetDateTime,
    /// The time of the revocation.
    revoked_at: Option<OffsetDateTime>,
    /// The time of the last use.
    last_used_at: Option<OffsetDateTime>,
}

impl Row {
    /// Makes the key of the row.
    fn into_info(self) -> AppKeyInfo {
        AppKeyInfo {
            id: AppKeyId::from_db(self.id),
            public_key: self.public_key,
            label: self.label,
            created_at: self.created_at,
            revoked_at: self.revoked_at,
            last_used_at: self.last_used_at,
        }
    }
}
