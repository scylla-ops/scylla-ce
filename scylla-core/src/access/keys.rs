//! The root keys of the tokens: one key signs, each known key verifies.

use std::collections::HashMap;

use biscuit_auth::{error, Algorithm, KeyPair, PrivateKey, PublicKey, RootKeyProvider};

/// The signing key of the tokens, and the public keys that still verify older tokens.
pub struct TokenKeys {
    /// The id of the signing key, written in each token.
    key_id: u32,
    /// The signing key.
    signer: KeyPair,
    /// The verification keys, by id. The signing key is one of them.
    verifiers: HashMap<u32, PublicKey>,
}

impl TokenKeys {
    /// Reads a hex Ed25519 private key and older public keys written as `id:hex`.
    pub fn new(private_hex: &str, key_id: u32, older: &[String]) -> Result<Self, String> {
        // Read the signing key. Its public key verifies the tokens of `key_id`.
        let private = PrivateKey::from_bytes_hex(private_hex, Algorithm::Ed25519)
            .map_err(|_| "the token private key must be an Ed25519 key in hex".to_owned())?;
        let signer = KeyPair::from(&private);
        let mut verifiers = HashMap::from([(key_id, signer.public())]);

        // Read the older public keys, for the tokens signed before a rotation.
        for entry in older {
            let (id, hex) = entry
                .split_once(':')
                .ok_or_else(|| format!("{entry:?} must be <id>:<hex>"))?;
            let id = id.parse().map_err(|_| format!("{id:?} is not a key id"))?;
            let key = PublicKey::from_bytes_hex(hex, Algorithm::Ed25519)
                .map_err(|_| format!("key {id} is not an Ed25519 public key in hex"))?;
            verifiers.insert(id, key);
        }
        Ok(Self {
            key_id,
            signer,
            verifiers,
        })
    }

    /// Makes a new random signing key.
    pub fn generate(key_id: u32) -> Self {
        let signer = KeyPair::new();
        let verifiers = HashMap::from([(key_id, signer.public())]);
        Self {
            key_id,
            signer,
            verifiers,
        }
    }

    /// Returns the private key in hex, to store as a secret.
    pub fn private_hex(&self) -> String {
        self.signer.private().to_bytes_hex()
    }

    /// Returns the public key in hex, to keep after a rotation.
    pub fn public_hex(&self) -> String {
        self.signer.public().to_bytes_hex()
    }

    /// Returns the id of the signing key.
    pub(super) fn key_id(&self) -> u32 {
        self.key_id
    }

    /// Returns the signing key.
    pub(super) fn signer(&self) -> &KeyPair {
        &self.signer
    }
}

impl RootKeyProvider for &TokenKeys {
    fn choose(&self, key_id: Option<u32>) -> Result<PublicKey, error::Format> {
        key_id
            .and_then(|id| self.verifiers.get(&id))
            .cloned()
            .ok_or(error::Format::UnknownPublicKey)
    }
}
