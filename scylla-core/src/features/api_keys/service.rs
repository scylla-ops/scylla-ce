//! The logic of the API keys: make, hash and revoke.

use scylla_domain::actors::ActorId;
use scylla_domain::api_keys::ApiKeyId;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use super::store;
use crate::Error;

/// The header that holds a key, as `Bearer <key>`.
pub const HEADER: &str = "authorization";

/// The start of each key, so a secret scanner can find a leaked key.
const PREFIX: &str = "scy_";

/// A new key. The server keeps only the hash of `secret`, so it shows the secret one time.
#[derive(Debug)]
pub struct NewKey {
    /// The id of the key, to revoke it.
    pub id: ApiKeyId,
    /// The key that the client sends.
    pub secret: String,
}

/// Makes a key for an actor. An unknown actor gives `NotFound("actor")`.
pub async fn create(pool: &PgPool, actor_id: ActorId) -> Result<NewKey, Error> {
    let secret = secret()?;
    let id = ApiKeyId::generate();
    let mut tx = pool.begin().await?;
    store::insert(&mut tx, id, actor_id, &hash(&secret)).await?;
    tx.commit().await?;
    Ok(NewKey { id, secret })
}

/// Revokes a key. A revoked key never authenticates again. An unknown key gives `NotFound("key")`.
pub async fn revoke(pool: &PgPool, id: ApiKeyId) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    let found = store::revoke(&mut tx, id).await?;
    tx.commit().await?;
    if found {
        Ok(())
    } else {
        Err(Error::NotFound("key"))
    }
}

/// Returns the SHA-256 hash of a key. A key has 256 random bits, so a fast hash is safe.
pub(super) fn hash(secret: &str) -> Vec<u8> {
    Sha256::digest(secret.as_bytes()).to_vec()
}

/// Makes a key: the prefix, then 32 random bytes in hex.
fn secret() -> Result<String, Error> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| std::io::Error::other(error.to_string()))?;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!("{PREFIX}{hex}"))
}
