//! The tests of the account mails: a text body and an HTML body, the token in the fragment of
//! each link, and at most one link each minute for a reset and for a change of email.

use std::time::Duration;

use scylla_core::SignUpMode;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_testkit::{text, TestApp};
use sqlx::PgPool;

use crate::app;
use crate::identity::accounts::{app_in, PASSWORD};

/// The time after which a mail that the server skips would have come.
const SETTLE: Duration = Duration::from_millis(300);

/// Moves the links of a user one minute and more into the past, so its cooldown ends.
async fn end_cooldown(app: &TestApp, user: ActorId) {
    sqlx::query!(
        "UPDATE one_time_tokens SET created_at = now() - interval '61 seconds' WHERE user_id = $1",
        user.to_db()
    )
    .execute(&app.pool)
    .await
    .unwrap();
}

/// Each link puts its token in the fragment, never in the query, in the text and HTML bodies.
#[sqlx::test(migrations = false)]
async fn each_link_has_its_token_in_the_fragment(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (new, reset, invited) = ("vic@example.com", "wes@example.com", "xia@example.com");
    app.user_with(reset, Some(PASSWORD)).await;

    // when
    app.sign_up(new, PASSWORD, "").await.unwrap();
    app.request_reset(reset).await.unwrap();
    app.create_invitation(owner, workspace, invited, "member")
        .await
        .unwrap();
    app.mails.wait(reset, 1).await;

    // then
    for (to, page) in [
        (new, "verify-email"),
        (reset, "reset-password"),
        (invited, "accept-invitation"),
    ] {
        let mail = app.mails.to(to).pop().unwrap();
        let token = app.mails.token(to, page);
        let link = format!("http://127.0.0.1:3000/{page}#token={token}");
        assert!(mail.body.contains(&link), "{}", mail.body);
        assert!(
            mail.html.contains(&format!(r#"href="{link}""#)),
            "{}",
            mail.html
        );
        assert!(!mail.body.contains("?token=") && !mail.html.contains("?token="));
    }
}

/// A mail has a text body and an HTML body. The HTML body escapes each value, and the text
/// body keeps it as it is.
#[sqlx::test(migrations = false)]
async fn a_mail_has_a_text_and_an_escaped_html_body(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let name = "<b>Tom & Jerry</b>";
    let workspace = app.create_workspace(owner, name).await.unwrap();
    let workspace = WorkspaceId::parse(text(&workspace.id)).unwrap();
    let email = "yan@example.com";

    // when
    app.create_invitation(owner, workspace, email, "viewer")
        .await
        .unwrap();

    // then
    let mail = app.mails.to(email).pop().unwrap();
    assert!(mail.body.starts_with("Join <b>Tom & Jerry</b>\n"));
    assert!(mail.html.starts_with("<!DOCTYPE html>"));
    assert!(mail.html.contains("&lt;b&gt;Tom &amp; Jerry&lt;/b&gt;"));
    assert!(!mail.html.contains(name));
}

/// An invitation names the person who invited and the role, and sends the logo as the only
/// image of the mail.
#[sqlx::test(migrations = false)]
async fn an_invitation_names_the_person_and_the_role(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.create_workspace(owner, "Acme Platform").await.unwrap();
    let workspace = WorkspaceId::parse(text(&workspace.id)).unwrap();
    let email = "ivy@example.com";

    // when
    app.create_invitation(owner, workspace, email, "viewer")
        .await
        .unwrap();

    // then
    let mail = app.mails.to(email).pop().unwrap();
    assert_eq!(
        mail.subject,
        "Test User invited you to Acme Platform on Scylla"
    );
    assert!(mail
        .body
        .contains("\nWorkspace: Acme Platform\nRole: Viewer\n"));
    let images: Vec<_> = mail
        .images
        .iter()
        .map(|image| &image.content_type)
        .collect();
    assert_eq!(images, ["image/png", "image/png"]);
}

/// A reset link names the time and the client of the request, and the notice of the new
/// password names the client of the change.
#[sqlx::test(migrations = false)]
async fn a_reset_names_the_client_of_each_request(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "kim@example.com";
    app.user_with(email, Some(PASSWORD)).await;

    // when
    app.request_reset(email).await.unwrap();
    app.mails.wait(email, 1).await;
    let token = app.mails.token(email, "reset-password");
    app.reset_password(&token, "a new password").await.unwrap();
    let mails = app.mails.wait(email, 2).await;

    // then
    assert_eq!(mails[0].subject, "Reset your Scylla password");
    assert!(mails[0].body.contains("\nRequested: "));
    assert_eq!(mails[1].subject, "Your Scylla password changed");
    for mail in &mails {
        assert!(mail.body.contains(" · 127.0.0.1\n"), "{}", mail.body);
    }
    assert!(mails[1]
        .body
        .contains("Reset your password: http://127.0.0.1:3000/forgot-password\n"));
}

/// The mail of a lock tells its end and the count of the wrong passwords.
#[sqlx::test(migrations = false)]
async fn a_lock_mail_tells_the_end_and_the_failures(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "lea@example.com";
    app.user_with(email, Some(PASSWORD)).await;

    // when
    for _ in 0..5 {
        app.sign_in(email, "a wrong password").await.unwrap_err();
    }
    let mails = app.mails.wait(email, 1).await;

    // then
    assert!(mails[0]
        .subject
        .starts_with("Your Scylla account is locked until "));
    assert!(mails[0].body.contains("\nLocked until: "));
    assert!(mails[0].body.contains("\nWrong passwords: 5 in a row\n"));
}

/// The alert of a reuse names the device of the session and the address of the old token, and a
/// new personal token tells its expiry.
#[sqlx::test(migrations = false)]
async fn a_notice_names_the_session_and_the_token(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "max@example.com";
    let user = app.user_with(email, Some(PASSWORD)).await;
    let session = app.sign_in(email, PASSWORD).await.unwrap();

    // when
    app.refresh(&session.refresh_token).await.unwrap();
    app.refresh(&session.refresh_token).await.unwrap_err();
    app.create_personal_token(user, "ci-deploy", None)
        .await
        .unwrap();
    let mails = app.mails.wait(email, 2).await;

    // then
    assert_eq!(mails[0].subject, "A Scylla session of your account closed");
    assert!(mails[0].body.contains("\nSession: "));
    assert!(mails[0].body.contains("\nLast address: 127.0.0.1\n"));
    assert!(mails[1].body.contains("\nName: ci-deploy\nExpires: "));
    assert!(mails[1]
        .body
        .contains("Review your tokens: http://127.0.0.1:3000/account/tokens\n"));
}

/// The server sends at most one reset link each minute for one account. After a minute, a new
/// link replaces the old one.
#[sqlx::test(migrations = false)]
async fn request_reset_sends_one_link_each_minute(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "zoe@example.com";
    let user = app.user_with(email, Some(PASSWORD)).await;
    app.request_reset(email).await.unwrap();
    let first = app.mails.wait(email, 1).await;
    let old = app.mails.token(email, "reset-password");

    // when
    app.request_reset(email).await.unwrap();
    tokio::time::sleep(SETTLE).await;
    let skipped = app.mails.to(email).len();
    end_cooldown(&app, user).await;
    app.request_reset(email).await.unwrap();
    app.mails.wait(email, 2).await;
    let new = app.mails.token(email, "reset-password");

    // then
    assert_eq!((first.len(), skipped), (1, 1));
    assert!(app.reset_password(&old, "an old link").await.is_err());
    assert!(app.reset_password(&new, "a new password").await.is_ok());
}

/// The server sends at most one link of an email change each minute for one account.
#[sqlx::test(migrations = false)]
async fn request_email_change_sends_one_link_each_minute(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (email, target) = ("abe@example.com", "abe.new@example.com");
    let user = app.user_with(email, Some(PASSWORD)).await;
    let token = app
        .sign_in(email, PASSWORD)
        .await
        .unwrap()
        .token
        .unwrap()
        .value;
    app.request_email_change(&token, target, PASSWORD)
        .await
        .unwrap();
    app.mails.wait(target, 1).await;

    // when
    app.request_email_change(&token, target, PASSWORD)
        .await
        .unwrap();
    tokio::time::sleep(SETTLE).await;
    let skipped = app.mails.to(target).len();
    end_cooldown(&app, user).await;
    app.request_email_change(&token, target, PASSWORD)
        .await
        .unwrap();
    let after = app.mails.wait(target, 2).await;

    // then
    assert_eq!((skipped, after.len()), (1, 2));
}

/// The server sends at most one link of an email change each minute to one email, whatever the
/// account that asks.
#[sqlx::test(migrations = false)]
async fn request_email_change_sends_one_link_each_minute_to_one_email(pool: PgPool) {
    // given
    let app = app(pool).await;
    let target = "shared.new@example.com";
    let mut tokens = Vec::new();
    for email in ["bea@example.com", "cid@example.com"] {
        app.user_with(email, Some(PASSWORD)).await;
        tokens.push(
            app.sign_in(email, PASSWORD)
                .await
                .unwrap()
                .token
                .unwrap()
                .value,
        );
    }

    // when
    for token in &tokens {
        app.request_email_change(token, target, PASSWORD)
            .await
            .unwrap();
    }
    app.mails.wait(target, 1).await;
    tokio::time::sleep(SETTLE).await;

    // then
    assert_eq!(app.mails.to(target).len(), 1);
}
