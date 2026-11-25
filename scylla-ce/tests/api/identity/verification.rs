//! The tests of a new link to verify an email: at a new sign-up with the email of an account that
//! waits for its check, on request with `ResendVerificationEmail` and its rate limits, and the
//! sender of the mails that the sign-in page shows.

use scylla_core::SignUpMode;
use scylla_domain::identity::actors::ActorId;
use scylla_testkit::TestApp;
use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::{app_in, PASSWORD};
use crate::identity::mails::{end_cooldown, SETTLE};
use crate::{app, assert_code, assert_invalid_field};

/// Another password of the tests.
const OTHER_PASSWORD: &str = "another long pass phrase";

/// Returns the user of an email.
async fn user_of(app: &TestApp, email: &str) -> ActorId {
    let id = sqlx::query_scalar!("SELECT user_id FROM user_emails WHERE email = $1", email)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    ActorId::from_db(id)
}

/// A new sign-up with the email of a pending account sends a new link, at most one each minute,
/// and keeps the password of the account.
#[sqlx::test(migrations = false)]
async fn sign_up_again_sends_a_new_link_to_a_pending_account(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;
    let email = "ana@example.com";
    app.sign_up(email, PASSWORD, "").await.unwrap();
    let first = app.mails.token(email, "verify-email").await;

    // when
    let soon = app.sign_up(email, OTHER_PASSWORD, "").await;
    let skipped = app.mails.to(email).await.len();
    end_cooldown(&app, user_of(&app, email).await).await;
    let later = app.sign_up(email, OTHER_PASSWORD, "").await;
    let mails = app.mails.to(email).await;
    let second = app.mails.token(email, "verify-email").await;
    let verified = app.verify_email(&second).await;

    // then
    assert!(soon.is_ok() && later.is_ok(), "{soon:?} {later:?}");
    assert_eq!((skipped, mails.len()), (1, 2));
    assert!(mails.iter().all(|mail| mail.subject == "Verify your email"));
    assert_ne!(first, second);
    assert!(verified.is_ok(), "{verified:?}");
    assert_code(
        app.sign_in(email, OTHER_PASSWORD).await,
        Code::Unauthenticated,
    );
    assert!(app.sign_in(email, PASSWORD).await.is_ok());
}

/// A request sends a new link to a pending account only, and each email gets the same answer.
#[sqlx::test(migrations = false)]
async fn resend_verification_email_sends_a_link_to_a_pending_account_only(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;
    let (pending, active, unknown) = ("pia@example.com", "act@example.com", "nobody@example.com");
    app.sign_up(pending, PASSWORD, "").await.unwrap();
    app.user_with(active, Some(PASSWORD)).await;
    end_cooldown(&app, user_of(&app, pending).await).await;

    // when
    let mut answers = Vec::new();
    for email in [pending, active, unknown] {
        answers.push(app.resend_verification(email).await);
    }
    let mails = app.mails.wait(pending, 2).await;
    tokio::time::sleep(SETTLE).await;
    let token = app.mails.token(pending, "verify-email").await;
    let verified = app.verify_email(&token).await;

    // then
    assert!(answers.iter().all(Result::is_ok), "{answers:?}");
    assert_eq!(mails.len(), 2);
    assert!(app.mails.to(active).await.is_empty() && app.mails.to(unknown).await.is_empty());
    assert!(verified.is_ok(), "{verified:?}");
    assert!(app.sign_in(pending, PASSWORD).await.is_ok());
}

/// The server sends at most one link each minute for one account.
#[sqlx::test(migrations = false)]
async fn resend_verification_email_sends_one_link_each_minute(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;
    let email = "ben@example.com";
    app.sign_up(email, PASSWORD, "").await.unwrap();

    // when
    app.resend_verification(email).await.unwrap();
    tokio::time::sleep(SETTLE).await;
    let skipped = app.mails.to(email).await.len();
    end_cooldown(&app, user_of(&app, email).await).await;
    app.resend_verification(email).await.unwrap();
    let after = app.mails.wait(email, 2).await;

    // then
    assert_eq!((skipped, after.len()), (1, 2));
}

/// One email gets 5 requests in one hour, and one IP address gets 20, then `RESOURCE_EXHAUSTED`.
#[sqlx::test(migrations = false)]
async fn resend_verification_email_limits_each_email_and_each_address(pool: PgPool) {
    // given
    let app = app(pool).await;
    let code = |result: Result<(), tonic::Status>| result.map_err(|status| status.code());

    // when
    let mut same = Vec::new();
    for _ in 0..6 {
        same.push(code(app.resend_verification("lim@example.com").await));
    }
    let mut others = Vec::new();
    for index in 0..15 {
        let email = format!("other{index}@example.com");
        others.push(code(app.resend_verification(&email).await));
    }

    // then
    assert_eq!(same[..5], [Ok(()); 5]);
    assert_eq!(same[5], Err(Code::ResourceExhausted));
    assert_eq!(others[..14], [Ok(()); 14]);
    assert_eq!(others[14], Err(Code::ResourceExhausted));
}

/// A text that is not an email is refused.
#[sqlx::test(migrations = false)]
async fn resend_verification_email_checks_the_email(pool: PgPool) {
    // given
    let app = app(pool).await;

    // when
    let refused = app.resend_verification("nope").await;

    // then
    assert_invalid_field(&refused.unwrap_err(), "email");
}

/// The sign-in methods name the address that the mails come from.
#[sqlx::test(migrations = false)]
async fn list_sign_in_methods_names_the_sender_of_the_mails(pool: PgPool) {
    // given
    let app = app(pool).await;

    // when
    let methods = app.sign_in_methods().await.unwrap();

    // then
    assert_eq!(methods.mail_sender, "no-reply@localhost");
}
