//! Unit tests of the signatures of the webhooks: GitHub and GitLab, good, bad and old.

use time::{Duration, OffsetDateTime};

use crate::infra::vcs::{
    check_github, check_gitlab, gitlab_signing_token, sign_github, sign_gitlab, GitLabDelivery,
    Hidden, SignatureError,
};

/// A body of a delivery.
const BODY: &[u8] = br#"{"ref":"refs/heads/main"}"#;

/// The id of a GitLab delivery.
const ID: &str = "f5e5f430-f57b-4e6e-9fac-d9128cd7232f";

/// The time of a GitLab delivery.
const SENT: i64 = 1_800_000_000;

/// Returns a time some seconds after the delivery.
fn after(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(SENT).expect("time") + Duration::seconds(seconds)
}

/// Makes the headers of a GitLab delivery.
fn delivery<'a>(id: &'a str, timestamp: &'a str, signature: &'a str) -> GitLabDelivery<'a> {
    GitLabDelivery {
        id: Some(id),
        timestamp: Some(timestamp),
        signature: Some(signature),
    }
}

/// A GitHub signature of the body with the secret passes, also with spaces around it.
#[test]
fn check_github_accepts_a_good_signature() {
    // given
    let signature = sign_github(b"secret", BODY);

    // when
    let checked = check_github(b"secret", Some(&format!(" {signature} ")), BODY);

    // then
    assert_eq!(checked, Ok(()));
    assert!(signature.starts_with("sha256="));
}

/// The signature of the example of the GitHub docs.
#[test]
fn sign_github_gives_the_signature_of_the_github_docs() {
    // when
    let signature = sign_github(b"It's a Secret to Everybody", b"Hello, World!");

    // then
    assert_eq!(
        signature,
        "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17"
    );
}

/// Another secret, another body, a missing prefix or no header is refused.
#[test]
fn check_github_refuses_a_bad_signature() {
    // given
    let signature = sign_github(b"secret", BODY);
    let bare = signature.trim_start_matches("sha256=").to_owned();

    // when
    let other_secret = check_github(b"other", Some(&signature), BODY);
    let other_body = check_github(b"secret", Some(&signature), b"{}");
    let no_prefix = check_github(b"secret", Some(&bare), BODY);
    let missing = check_github(b"secret", None, BODY);

    // then
    assert_eq!(other_secret, Err(SignatureError::Bad));
    assert_eq!(other_body, Err(SignatureError::Bad));
    assert_eq!(no_prefix, Err(SignatureError::Bad));
    assert_eq!(missing, Err(SignatureError::Missing));
}

/// A GitLab signature over `id.timestamp.body` passes within the window, also next to another
/// signature in the list.
#[test]
fn check_gitlab_accepts_a_good_signature() {
    // given
    let token = gitlab_signing_token();
    let timestamp = SENT.to_string();
    let signature = sign_gitlab(&token, ID, &timestamp, BODY);
    let list = format!("v1,AAAA {signature}");

    // when
    let checked = check_gitlab(&token, &delivery(ID, &timestamp, &list), BODY, after(299));

    // then
    assert_eq!(checked, Ok(()));
    assert!(token.expose().starts_with("whsec_"));
}

/// The signature of the example of Standard Webhooks, which GitLab follows: the key is the
/// base64 after `whsec_`.
#[test]
fn sign_gitlab_gives_the_signature_of_standard_webhooks() {
    // given
    let token = Hidden::new("whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw");
    let body = br#"{"test": 2432232314}"#;

    // when
    let signature = sign_gitlab(&token, "msg_p5jXN8AQM9LWM0D4loKWxJek", "1614265330", body);

    // then
    assert_eq!(signature, "v1,g0hM9SsE+OTPJTGt/tmIKtSyZlE3uFJELVlNIOLJ1OE=");
}

/// Another token, another body, another id or a missing header is refused.
#[test]
fn check_gitlab_refuses_a_bad_signature() {
    // given
    let token = gitlab_signing_token();
    let timestamp = SENT.to_string();
    let signature = sign_gitlab(&token, ID, &timestamp, BODY);
    let headers = delivery(ID, &timestamp, &signature);

    // when
    let other_token = check_gitlab(&gitlab_signing_token(), &headers, BODY, after(0));
    let other_body = check_gitlab(&token, &headers, b"{}", after(0));
    let other_id = check_gitlab(
        &token,
        &delivery("id-2", &timestamp, &signature),
        BODY,
        after(0),
    );
    let missing = check_gitlab(&token, &GitLabDelivery::default(), BODY, after(0));

    // then
    assert_eq!(other_token, Err(SignatureError::Bad));
    assert_eq!(other_body, Err(SignatureError::Bad));
    assert_eq!(other_id, Err(SignatureError::Bad));
    assert_eq!(missing, Err(SignatureError::Missing));
}

/// A delivery more than 5 minutes old, or more than 5 minutes in the future, is refused.
#[test]
fn check_gitlab_refuses_an_old_delivery() {
    // given
    let token = gitlab_signing_token();
    let timestamp = SENT.to_string();
    let signature = sign_gitlab(&token, ID, &timestamp, BODY);
    let headers = delivery(ID, &timestamp, &signature);

    // when
    let old = check_gitlab(&token, &headers, BODY, after(301));
    let future = check_gitlab(&token, &headers, BODY, after(-301));

    // then
    assert_eq!(old, Err(SignatureError::Old));
    assert_eq!(future, Err(SignatureError::Old));
}
