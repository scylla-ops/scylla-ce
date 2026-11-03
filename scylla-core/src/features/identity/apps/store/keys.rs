//! The SQL of the `app_keys` table.

use scylla_domain::identity::apps::{AppId, AppKeyId, AppKeyInfo};
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

/// Inserts a public key of an App and returns it.
pub(crate) async fn insert(
    tx: &mut Tx<'_>,
    app: AppId,
    public_key: &[u8],
) -> Result<AppKeyInfo, sqlx::Error> {
    let id = AppKeyId::generate();
    let created_at = sqlx::query_scalar!(
        "INSERT INTO app_keys (id, app_id, public_key) VALUES ($1, $2, $3) RETURNING created_at",
        id.to_db(),
        app.to_db(),
        public_key,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(AppKeyInfo {
        id,
        public_key: public_key.to_vec(),
        created_at,
        revoked_at: None,
    })
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
    let rows = sqlx::query!(
        "SELECT id, public_key, created_at, revoked_at FROM app_keys
         WHERE app_id = $1 ORDER BY id",
        app.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| AppKeyInfo {
            id: AppKeyId::from_db(row.id),
            public_key: row.public_key,
            created_at: row.created_at,
            revoked_at: row.revoked_at,
        })
        .collect())
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
