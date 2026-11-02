//! The hashes of the passwords: Argon2id with its default parameters, off the async threads.

use std::sync::OnceLock;

use argon2::password_hash::phc::PasswordHash;
use argon2::{Argon2, PasswordHasher, PasswordVerifier};

use crate::Error;

/// Hashes a password to a PHC string, with a new random salt.
pub(crate) async fn hash(password: String) -> Result<String, Error> {
    let hash = tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|hash| hash.to_string())
    })
    .await
    .map_err(|error| Error::Internal(format!("the password hash stopped: {error}")))?;
    hash.map_err(|error| Error::Internal(format!("the password hash failed: {error}")))
}

/// Returns `true` if the password matches the hash. With no hash, it spends the same time and
/// returns `false`, so the answer does not tell if the account has a password.
pub(crate) async fn verify(password: String, hash: Option<String>) -> bool {
    let checked = tokio::task::spawn_blocking(move || {
        let stored = hash.as_deref().unwrap_or_else(|| decoy());
        let matches = PasswordHash::new(stored)
            .map(|parsed| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &parsed)
                    .is_ok()
            })
            .unwrap_or(false);
        matches && hash.is_some()
    })
    .await;
    checked.unwrap_or(false)
}

/// Returns the hash of a random password, made one time, for the checks of an account with no
/// password.
fn decoy() -> &'static str {
    static DECOY: OnceLock<String> = OnceLock::new();
    DECOY.get_or_init(|| {
        Argon2::default()
            .hash_password(crate::secret::token().as_bytes())
            .map(|hash| hash.to_string())
            .unwrap_or_default()
    })
}
