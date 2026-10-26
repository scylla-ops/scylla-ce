//! The pure parts of a reset: the token, the link and the message.

use super::*;
use crate::domain::clock;
use crate::domain::ids::UserId;
use crate::domain::user::{Email, RESET_TOKEN_LEN, ResetToken};

#[test]
fn a_minted_token_is_43_characters_of_base64url_and_never_repeats() {
    let a = mint_reset_token().unwrap();
    let b = mint_reset_token().unwrap();
    assert_eq!(a.as_str().len(), RESET_TOKEN_LEN);
    assert_ne!(a, b);
}

#[test]
fn a_link_starts_with_the_public_url_or_is_relative() {
    let token = ResetToken::new("abcdefghijklmnopqrstuvwxyzABCDEFGHIJ0123-_Z").unwrap();
    assert_eq!(
        ResetLinks::new(Some("http://127.0.0.1:8080/")).link(&token),
        "http://127.0.0.1:8080/reset-password#token=abcdefghijklmnopqrstuvwxyzABCDEFGHIJ0123-_Z"
    );
    assert_eq!(
        ResetLinks::new(None).link(&token),
        "/reset-password#token=abcdefghijklmnopqrstuvwxyzABCDEFGHIJ0123-_Z"
    );
}

#[test]
fn a_message_never_prints_its_link() {
    let message = PasswordResetMessage {
        user_id: UserId::new("kevin"),
        email: Email::new("kevin@example.com").unwrap(),
        username: crate::domain::user::Username::new("kevin").unwrap(),
        display_name: None,
        link: "https://x/reset-password#token=secret".into(),
        expires_at: clock::now(),
    };
    let printed = format!("{message:?}");
    assert!(!printed.contains("secret"), "{printed}");
    assert!(!printed.contains("kevin@example.com"), "{printed}");
}
