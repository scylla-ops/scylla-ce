//! The tests of the self-service of an account: the profile, the change of the email, and the
//! sign-out of the other sessions.

use std::time::Duration;

use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::{assert_refused, PASSWORD};
use crate::{app, assert_code, assert_invalid_field};

/// Signs in a new user with a password, and returns its access token.
async fn signed_in(app: &scylla_testkit::TestApp, email: &str) -> String {
    app.user_with(email, Some(PASSWORD)).await;
    app.sign_in(email, PASSWORD)
        .await
        .unwrap()
        .token
        .unwrap()
        .value
}

/// A user changes its display name, and a field that is not set does not change.
#[sqlx::test(migrations = false)]
async fn update_profile_changes_the_display_name(pool: PgPool) {
    // given
    let app = app(pool).await;
    let token = signed_in(&app, "pia@example.com").await;

    // when
    let renamed = app.update_profile(&token, Some("  Pia R.  "), None).await;
    let empty = app.update_profile(&token, Some(" "), None).await;
    let same = app.update_profile(&token, None, None).await.unwrap();

    // then
    assert_eq!(renamed.unwrap().display_name, "Pia R.");
    assert_invalid_field(&empty.unwrap_err(), "display_name");
    assert_eq!(same.display_name, "Pia R.");
}

/// A user closes each other session: their tokens stop, and the session of the request stays.
#[sqlx::test(migrations = false)]
async fn revoke_other_sessions_keeps_the_current_one(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "quin@example.com";
    let this = signed_in(&app, email).await;
    let other = app.sign_in(email, PASSWORD).await.unwrap();

    // when
    app.revoke_other_sessions(&this).await.unwrap();

    // then
    assert_eq!(app.list_sessions(&this).await.unwrap().len(), 1);
    assert_code(
        app.get_me(&other.token.unwrap().value).await,
        Code::Unauthenticated,
    );
    assert_code(
        app.refresh(&other.refresh_token).await,
        Code::Unauthenticated,
    );
}

/// The link goes to the new email. Its token makes the new email the primary verified email,
/// one time only, and the old email gets a notice and stops working for a sign-in.
#[sqlx::test(migrations = false)]
async fn an_email_change_needs_the_link_of_the_new_email(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (old, new) = ("rob@example.com", "robert@example.com");
    let token = signed_in(&app, old).await;

    // when
    app.request_email_change(&token, new, PASSWORD)
        .await
        .unwrap();
    app.mails.wait(new, 1).await;
    let link = app.mails.token(new, "change-email");
    let changed = app.confirm_email_change(&token, &link).await.unwrap();
    let again = app.confirm_email_change(&token, &link).await;

    // then
    assert_eq!(changed.email, new);
    assert_invalid_field(&again.unwrap_err(), "token");
    let notice = app.mails.to(old);
    assert_eq!(notice.last().unwrap().subject, "Your Scylla email changed");
    assert!(notice.last().unwrap().body.contains(new));
    assert!(app.sign_in(new, PASSWORD).await.is_ok());
    assert_code(app.sign_in(old, PASSWORD).await, Code::Unauthenticated);
}

/// The token of a link works only for its user, and an email of another account gets the same
/// answer and no link.
#[sqlx::test(migrations = false)]
async fn an_email_change_keeps_the_emails_of_other_accounts(pool: PgPool) {
    // given
    let app = app(pool).await;
    let sam = signed_in(&app, "sam@example.com").await;
    let tia = signed_in(&app, "tia@example.com").await;
    app.request_email_change(&sam, "sam.new@example.com", PASSWORD)
        .await
        .unwrap();
    app.mails.wait("sam.new@example.com", 1).await;
    let link = app.mails.token("sam.new@example.com", "change-email");

    // when
    let other_user = app.confirm_email_change(&tia, &link).await;
    let used = app
        .request_email_change(&tia, "sam@example.com", PASSWORD)
        .await;
    let not_an_email = app.request_email_change(&tia, "nope", PASSWORD).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    // then
    assert_invalid_field(&other_user.unwrap_err(), "token");
    assert!(used.is_ok(), "{used:?}");
    assert_eq!(app.mails.to("sam@example.com").len(), 0);
    assert_invalid_field(&not_an_email.unwrap_err(), "new_email");
    assert!(app.confirm_email_change(&sam, &link).await.is_ok());
}

/// A new email that an account got after the link is refused at the confirmation.
#[sqlx::test(migrations = false)]
async fn confirm_email_change_refuses_an_email_in_use(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (target, token) = (
        "uma.new@example.com",
        signed_in(&app, "uma@example.com").await,
    );
    app.request_email_change(&token, target, PASSWORD)
        .await
        .unwrap();
    app.mails.wait(target, 1).await;
    let link = app.mails.token(target, "change-email");
    app.user_with(target, None).await;

    // when
    let confirmed = app.confirm_email_change(&token, &link).await;

    // then
    assert_refused(confirmed, "EMAIL_IN_USE");
}

/// A change of email needs the current password: a wrong password gets `INVALID_ARGUMENT` and
/// no link.
#[sqlx::test(migrations = false)]
async fn request_email_change_needs_the_current_password(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (target, token) = (
        "val.new@example.com",
        signed_in(&app, "val@example.com").await,
    );

    // when
    let wrong = app
        .request_email_change(&token, target, "not the password")
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let skipped = app.mails.to(target).len();
    app.request_email_change(&token, target, PASSWORD)
        .await
        .unwrap();

    // then
    assert_invalid_field(&wrong.unwrap_err(), "password");
    assert_eq!(skipped, 0);
    assert_eq!(app.mails.wait(target, 1).await.len(), 1);
}

/// A change of email stops each open link of the account: a reset link that went to the old
/// email does not work after the change.
#[sqlx::test(migrations = false)]
async fn an_email_change_stops_the_links_of_the_old_email(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (old, new) = ("wil@example.com", "wil.new@example.com");
    let token = signed_in(&app, old).await;
    app.request_reset(old).await.unwrap();
    app.mails.wait(old, 1).await;
    let reset = app.mails.token(old, "reset-password");
    app.request_email_change(&token, new, PASSWORD)
        .await
        .unwrap();
    app.mails.wait(new, 1).await;
    let link = app.mails.token(new, "change-email");

    // when
    app.confirm_email_change(&token, &link).await.unwrap();
    let after = app.reset_password(&reset, "an attacker password").await;

    // then
    assert_invalid_field(&after.unwrap_err(), "token");
    assert!(app.sign_in(new, PASSWORD).await.is_ok());
}
