//! Random tokens, their hashes, and the encryption of the secrets that the server must read
//! back.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chacha20poly1305::aead::{Aead, Generate, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use sha2::{Digest, Sha256};

/// The number of random bytes in a token.
const TOKEN_BYTES: usize = 32;

/// The number of bytes of a nonce of XChaCha20-Poly1305.
const NONCE_BYTES: usize = 24;

/// Makes a random token: 32 bytes in URL-safe base64, with no padding.
pub fn token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).expect("the system gives random bytes");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Returns the SHA-256 hash of a token. The database keeps the hash, never the token.
pub fn hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// Returns the PKCE challenge of a verifier: the URL-safe base64 of its SHA-256 hash.
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Encrypts the secrets that the server must read back, with the key of the settings and
/// XChaCha20-Poly1305: the client secret of an identity provider, and the value of a secret of a
/// project, bound to its id.
pub struct Sealer {
    /// The cipher with the key.
    cipher: XChaCha20Poly1305,
}

impl Sealer {
    /// Makes a sealer from a key of 32 bytes in hex.
    pub fn from_hex(key: &str) -> Result<Self, String> {
        let bytes = hex::decode(key.trim()).map_err(|_| "must be hex".to_owned())?;
        let cipher = XChaCha20Poly1305::new_from_slice(&bytes)
            .map_err(|_| "must be 32 bytes in hex".to_owned())?;
        Ok(Self { cipher })
    }

    /// Makes a sealer with a new random key, for a test or for `secret keygen`.
    pub fn generate() -> (Self, String) {
        let mut key = [0u8; 32];
        getrandom::fill(&mut key).expect("the system gives random bytes");
        let cipher = XChaCha20Poly1305::new_from_slice(&key).expect("a key of 32 bytes");
        (Self { cipher }, hex::encode(key))
    }

    /// Encrypts a text with no associated data. The result starts with its random nonce.
    pub fn seal(&self, plain: &str) -> Vec<u8> {
        self.seal_with(plain.as_bytes(), b"")
    }

    /// Decrypts a text of `seal`. A wrong key or a changed text gives an error.
    pub fn open(&self, sealed: &[u8]) -> Result<String, String> {
        let plain = self.open_with(sealed, b"")?;
        String::from_utf8(plain).map_err(|_| unreadable())
    }

    /// Encrypts bytes bound to associated data, for example the id of their row. The result
    /// starts with its random nonce, and opens only with the same associated data.
    pub fn seal_with(&self, plain: &[u8], associated: &[u8]) -> Vec<u8> {
        let nonce = XNonce::generate();
        let payload = Payload {
            msg: plain,
            aad: associated,
        };
        let sealed = self
            .cipher
            .encrypt(&nonce, payload)
            .expect("the encryption of a short text");
        [nonce.as_slice(), sealed.as_slice()].concat()
    }

    /// Decrypts bytes of `seal_with`. A wrong key, other associated data or changed bytes give
    /// an error.
    pub fn open_with(&self, sealed: &[u8], associated: &[u8]) -> Result<Vec<u8>, String> {
        if sealed.len() < NONCE_BYTES {
            return Err(unreadable());
        }
        let (nonce, text) = sealed.split_at(NONCE_BYTES);
        let nonce = XNonce::try_from(nonce).map_err(|_| unreadable())?;
        let payload = Payload {
            msg: text,
            aad: associated,
        };
        self.cipher
            .decrypt(&nonce, payload)
            .map_err(|_| unreadable())
    }
}

/// The error of a secret that does not open.
fn unreadable() -> String {
    "the secret cannot be read with this key".to_owned()
}

#[cfg(test)]
mod tests {
    //! Unit tests of the tokens and of the sealer.

    use super::{hash, pkce_challenge, token, Sealer};

    /// Two tokens differ, and a hash is the same for the same token.
    #[test]
    fn tokens_are_random_and_hashes_are_stable() {
        // given
        let (first, second) = (token(), token());

        // when
        let hashes = (hash(&first), hash(&first));

        // then
        assert_ne!(first, second);
        assert_eq!(first.len(), 43);
        assert_eq!(hashes.0, hashes.1);
    }

    /// The challenge of the example verifier of RFC 7636 is the one of the RFC.
    #[test]
    fn the_pkce_challenge_follows_the_rfc() {
        // given
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

        // when
        let challenge = pkce_challenge(verifier);

        // then
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    /// A sealed text opens with its key only, and a changed text does not open.
    #[test]
    fn a_sealed_text_opens_with_its_key_only() {
        // given
        let (sealer, _) = Sealer::generate();
        let (other, _) = Sealer::generate();
        let mut sealed = sealer.seal("client secret");

        // when
        let opened = sealer.open(&sealed);
        let with_other = other.open(&sealed);
        sealed[30] ^= 1;
        let changed = sealer.open(&sealed);

        // then
        assert_eq!(opened.as_deref(), Ok("client secret"));
        assert!(with_other.is_err() && changed.is_err());
    }

    /// Bytes sealed with associated data open with the same data only, and not as a text of
    /// `seal`.
    #[test]
    fn sealed_bytes_open_with_their_associated_data_only() {
        // given
        let (sealer, _) = Sealer::generate();
        let sealed = sealer.seal_with(b"value", b"first id");

        // when
        let opened = sealer.open_with(&sealed, b"first id");
        let other_data = sealer.open_with(&sealed, b"other id");
        let no_data = sealer.open(&sealed);

        // then
        assert_eq!(opened.as_deref(), Ok(&b"value"[..]));
        assert!(other_data.is_err() && no_data.is_err());
    }
}
