//! The signature check of a delivery.

use super::*;

const KEY: &str = "key";
const MSG: &[u8] = b"The quick brown fox jumps over the lazy dog";
const SIG: &str = "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8";

#[test]
fn accepts_correct_signature_with_and_without_prefix() {
    let digest = Some(SIG.to_string());
    assert_eq!(verified_digest(KEY, MSG, SIG), digest);
    assert_eq!(verified_digest(KEY, MSG, &format!("sha256={SIG}")), digest);
    assert_eq!(verified_digest(KEY, MSG, &format!("sha256= {SIG}")), digest);
}

#[test]
fn uppercase_hex_and_prefix_verify() {
    let upper = SIG.to_uppercase();
    let digest = Some(SIG.to_string());
    assert_eq!(verified_digest(KEY, MSG, &upper), digest);
    assert_eq!(
        verified_digest(KEY, MSG, &format!("SHA256={upper}")),
        digest
    );
    assert_eq!(verified_digest(KEY, MSG, &format!("Sha256={SIG}")), digest);
}

#[test]
fn rejects_tampered_body_wrong_key_and_garbage() {
    assert_eq!(verified_digest(KEY, b"tampered", SIG), None);
    assert_eq!(verified_digest("wrong-key", MSG, SIG), None);
    assert_eq!(verified_digest(KEY, MSG, "not-hex"), None);
    assert_eq!(verified_digest(KEY, MSG, &SIG[..62]), None);
    assert_eq!(verified_digest(KEY, MSG, "sha256="), None);
    assert_eq!(verified_digest(KEY, MSG, ""), None);
}
