//! The tests of the accounts with a password: sign-up, the check of the email, the sign-in, the
//! reset and the change of a password.

use scylla_core::api::ServeOptions;
use scylla_core::SignUpMode;
use scylla_testkit::TestApp;
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

use crate::{app, assert_code, assert_invalid_field};

/// A password of the tests.
pub(crate) const PASSWORD: &str = "correct horse battery staple";

/// Another password of the tests.
const NEW_PASSWORD: &str = "another long pass phrase";

/// Starts the Community edition with a sign-up mode.
pub(crate) async fn app_in(pool: PgPool, mode: SignUpMode) -> TestApp {
    let mut options = ServeOptions::default();
    options.accounts.sign_up_mode = mode;
    TestApp::spawn_with(pool, scylla_ce::edition(), options).await
}

/// Checks that `status` is the refusal of an account rule with its code.
pub(crate) fn assert_refused<T: std::fmt::Debug>(result: Result<T, Status>, reason: &str) {
    let status = result.expect_err("the call must fail");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    let info = status.get_details_error_info().expect("ErrorInfo detail");
    assert_eq!(info.reason, reason);
}

/// A sign-up makes a pending user that cannot sign in. The link of the mail activates it, one
/// time only.
#[sqlx::test(migrations = false)]
async fn sign_up_needs_the_link_of_the_mail(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;
    let email = "new@example.com";
    app.sign_up(email, PASSWORD, "").await.unwrap();

    // when
    let before = app.sign_in(email, PASSWORD).await;
    let token = app.mails.token(email, "verify-email");
    let verified = app.verify_email(&token).await;
    let again = app.verify_email(&token).await;
    let after = app.sign_in(email, PASSWORD).await.unwrap();

    // then
    assert_refused(before, "EMAIL_NOT_VERIFIED");
    assert!(verified.is_ok(), "{verified:?}");
    assert_invalid_field(&again.unwrap_err(), "token");
    let me = app.get_me(&after.token.unwrap().value).await.unwrap();
    assert_eq!((me.email.as_str(), me.status.as_str()), (email, "active"));
}

/// A known email gets the same answer, its owner gets a mail, and its password stays.
#[sqlx::test(migrations = false)]
async fn sign_up_with_a_known_email_sends_a_mail(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;
    let email = "taken@example.com";
    app.user_with(email, Some(PASSWORD)).await;

    // when
    let result = app.sign_up(email, NEW_PASSWORD, "").await;

    // then
    assert!(result.is_ok(), "{result:?}");
    let mails = app.mails.to(email);
    assert_eq!(mails.last().unwrap().subject, "Your Scylla account");
    assert_code(
        app.sign_in(email, NEW_PASSWORD).await,
        Code::Unauthenticated,
    );
    assert!(app.sign_in(email, PASSWORD).await.is_ok());
}

/// The default mode needs an invitation, and the closed mode refuses each sign-up.
#[sqlx::test(migrations = false)]
async fn sign_up_follows_the_mode(pool: PgPool) {
    // given
    let invite_only = app(pool.clone()).await;
    let closed = app_in(pool, SignUpMode::Closed).await;

    // when
    let without_invitation = invite_only.sign_up("a@example.com", PASSWORD, "").await;
    let when_closed = closed.sign_up("b@example.com", PASSWORD, "").await;

    // then
    assert_refused(without_invitation, "INVITATION_REQUIRED");
    assert_refused(when_closed, "SIGN_UP_CLOSED");
}

/// A short password and a text that is not an email are refused.
#[sqlx::test(migrations = false)]
async fn sign_up_checks_the_values(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;

    // when
    let short = app.sign_up("c@example.com", "short", "").await;
    let not_an_email = app.sign_up("nope", PASSWORD, "").await;

    // then
    assert_invalid_field(&short.unwrap_err(), "password");
    assert_invalid_field(&not_an_email.unwrap_err(), "email");
}

/// A password has 8 to 128 characters.
#[sqlx::test(migrations = false)]
async fn sign_up_checks_the_length_of_the_password(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;

    // when
    let seven = app.sign_up("p7@example.com", &"p".repeat(7), "").await;
    let eight = app.sign_up("p8@example.com", &"p".repeat(8), "").await;
    let max = app.sign_up("p128@example.com", &"p".repeat(128), "").await;
    let over = app.sign_up("p129@example.com", &"p".repeat(129), "").await;

    // then
    assert_invalid_field(&seven.unwrap_err(), "password");
    assert!(eight.is_ok() && max.is_ok(), "{eight:?} {max:?}");
    assert_invalid_field(&over.unwrap_err(), "password");
}

/// A wrong password and an unknown email get the same answer. After 5 failures, even the right
/// password is refused for a while.
#[sqlx::test(migrations = false)]
async fn sign_in_refuses_the_same_way_and_locks(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "bob@example.com";
    app.user_with(email, Some(PASSWORD)).await;

    // when
    let wrong = app.sign_in(email, NEW_PASSWORD).await.unwrap_err();
    let unknown = app
        .sign_in("nobody@example.com", PASSWORD)
        .await
        .unwrap_err();
    for _ in 0..4 {
        app.sign_in(email, NEW_PASSWORD).await.unwrap_err();
    }
    let locked = app.sign_in(email, PASSWORD).await;

    // then
    assert_eq!(wrong.code(), Code::Unauthenticated);
    assert_eq!(
        (wrong.code(), wrong.message()),
        (unknown.code(), unknown.message())
    );
    assert_code(locked, Code::Unauthenticated);
}

/// A failure while the account is locked does not count, so 50 failures keep the same answer
/// and the lock. A long count of old failures still gives a lock of one hour at most.
#[sqlx::test(migrations = false)]
async fn sign_in_stays_locked_after_many_failures(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "lou@example.com";
    let user = app.user_with(email, Some(PASSWORD)).await;

    // when
    let mut codes = Vec::new();
    for _ in 0..50 {
        codes.push(app.sign_in(email, NEW_PASSWORD).await.unwrap_err().code());
    }
    let locked = app.sign_in(email, PASSWORD).await;
    let counted = sqlx::query_scalar!(
        "SELECT failed_attempts FROM passwords WHERE user_id = $1",
        user.to_db()
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    sqlx::query!(
        "UPDATE passwords SET failed_attempts = 60, locked_until = now() - interval '1 second'
         WHERE user_id = $1",
        user.to_db()
    )
    .execute(&app.pool)
    .await
    .unwrap();
    let after_many = app.sign_in(email, NEW_PASSWORD).await;

    // then
    assert_eq!(codes, vec![Code::Unauthenticated; 50]);
    assert_code(locked, Code::Unauthenticated);
    assert_eq!(counted, 5);
    assert_code(after_many, Code::Unauthenticated);
}

/// The link of a reset sets a new password and closes each session. An unknown email gets the
/// same answer and no mail.
#[sqlx::test(migrations = false)]
async fn reset_password_closes_the_sessions(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "carol@example.com";
    app.user_with(email, Some(PASSWORD)).await;
    let session = app.sign_in(email, PASSWORD).await.unwrap();

    // when
    app.request_reset(email).await.unwrap();
    let unknown = app.request_reset("nobody@example.com").await;
    app.mails.wait(email, 1).await;
    let token = app.mails.token(email, "reset-password");
    app.reset_password(&token, NEW_PASSWORD).await.unwrap();

    // then
    assert!(unknown.is_ok() && app.mails.to("nobody@example.com").is_empty());
    assert_code(
        app.refresh(&session.refresh_token).await,
        Code::Unauthenticated,
    );
    assert_code(app.sign_in(email, PASSWORD).await, Code::Unauthenticated);
    assert!(app.sign_in(email, NEW_PASSWORD).await.is_ok());
}

/// A change needs the current password, and closes the other sessions only.
#[sqlx::test(migrations = false)]
async fn change_password_closes_the_other_sessions(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "dave@example.com";
    app.user_with(email, Some(PASSWORD)).await;
    let this = app.sign_in(email, PASSWORD).await.unwrap();
    let other = app.sign_in(email, PASSWORD).await.unwrap();
    let token = this.token.unwrap().value;

    // when
    let wrong = app
        .change_password(&token, NEW_PASSWORD, NEW_PASSWORD)
        .await;
    app.change_password(&token, PASSWORD, NEW_PASSWORD)
        .await
        .unwrap();

    // then
    assert_invalid_field(&wrong.unwrap_err(), "current_password");
    assert!(app.refresh(&this.refresh_token).await.is_ok());
    assert_code(
        app.refresh(&other.refresh_token).await,
        Code::Unauthenticated,
    );
    assert!(app.sign_in(email, NEW_PASSWORD).await.is_ok());
}
