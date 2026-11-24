//! The tests of the kinds that a person must know: a row and a mail for each one, also for an own
//! action, and no secret in a mail.

use sqlx::PgPool;

use super::{kinds_of, row_of};
use crate::app;
use crate::identity::accounts::PASSWORD;

/// The email of the account of these tests.
const EMAIL: &str = "secure@example.com";

/// The new password of these tests.
const NEW_PASSWORD: &str = "another long pass phrase";

/// A change of a password makes a row and a mail, for an own action too.
#[sqlx::test(migrations = false)]
async fn a_password_change_makes_a_row_and_a_mail(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user_with(EMAIL, Some(PASSWORD)).await;
    let session = app.sign_in(EMAIL, PASSWORD).await.unwrap();
    let token = session.token.unwrap().value;

    // when
    app.change_password(&token, PASSWORD, NEW_PASSWORD)
        .await
        .unwrap();
    let mails = app.mails.wait(EMAIL, 1).await;

    // then
    assert_eq!(kinds_of(&app, user).await, ["security.password_changed"]);
    assert!(mails[0].subject.contains("password changed"));
    assert!(!mails[0].body.contains(NEW_PASSWORD));
}

/// A reset of a password makes a row and a mail, and the link of the reset works one time.
#[sqlx::test(migrations = false)]
async fn a_reset_makes_a_row_and_a_mail(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user_with(EMAIL, Some(PASSWORD)).await;
    app.request_reset(EMAIL).await.unwrap();
    app.mails.wait(EMAIL, 1).await;
    let link = app.mails.token(EMAIL, "reset-password");

    // when
    app.reset_password(&link, NEW_PASSWORD).await.unwrap();
    let mails = app.mails.wait(EMAIL, 2).await;

    // then
    assert_eq!(kinds_of(&app, user).await, ["security.password_changed"]);
    assert!(mails[1].subject.contains("password changed"));
}

/// A second use of a refresh token closes the session, and makes a row and a mail.
#[sqlx::test(migrations = false)]
async fn a_session_reuse_makes_a_row_and_a_mail(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user_with(EMAIL, Some(PASSWORD)).await;
    let session = app.sign_in(EMAIL, PASSWORD).await.unwrap();
    app.refresh(&session.refresh_token).await.unwrap();

    // when
    let reuse = app.refresh(&session.refresh_token).await;
    let again = app.refresh(&session.refresh_token).await;
    let mails = app.mails.wait(EMAIL, 1).await;

    // then
    assert!(reuse.is_err());
    assert!(again.is_err());
    assert_eq!(kinds_of(&app, user).await, ["security.session_reuse"]);
    assert_eq!(mails.len(), 1);
    assert!(!mails[0].body.contains(&session.refresh_token));
}

/// A lockout makes one row and one mail for each hour.
#[sqlx::test(migrations = false)]
async fn a_lockout_makes_one_row_and_one_mail(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let user = app.user_with(EMAIL, Some(PASSWORD)).await;

    // when
    for _ in 0..10 {
        app.sign_in(EMAIL, "a wrong pass phrase").await.ok();
    }
    let mails = app.mails.wait(EMAIL, 1).await;
    sqlx::query!(
        "UPDATE passwords SET locked_until = NULL, failed_attempts = 0 WHERE user_id = $1",
        user.to_db()
    )
    .execute(&pool)
    .await
    .unwrap();
    for _ in 0..10 {
        app.sign_in(EMAIL, "a wrong pass phrase").await.ok();
    }

    // then
    assert_eq!(kinds_of(&app, user).await, ["security.account_locked"]);
    assert!(mails[0].subject.contains("locked"));
    assert_eq!(app.mails.to(EMAIL).len(), 1);
}

/// A new personal token makes a row with its name and a mail, and never holds its secret.
#[sqlx::test(migrations = false)]
async fn a_new_personal_token_makes_a_row_and_a_mail(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user_with(EMAIL, Some(PASSWORD)).await;

    // when
    let created = app.create_personal_token(user, "ci", None).await.unwrap();
    let mails = app.mails.wait(EMAIL, 1).await;
    let row = row_of(&app, user, "token.created").await;

    // then
    assert_eq!(row.params.get("name").map(String::as_str), Some("ci"));
    assert!(mails[0].subject.contains("ci"));
    assert!(!mails[0].body.contains(&created.secret));
}

/// A change of an email makes a row, with no address in it.
#[sqlx::test(migrations = false)]
async fn an_email_change_makes_a_row(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user_with(EMAIL, Some(PASSWORD)).await;
    let session = app.sign_in(EMAIL, PASSWORD).await.unwrap();
    let token = session.token.unwrap().value;
    let new = "moved@example.com";
    app.request_email_change(&token, new, PASSWORD)
        .await
        .unwrap();
    app.mails.wait(new, 1).await;
    let link = app.mails.token(new, "change-email");

    // when
    app.confirm_email_change(&token, &link).await.unwrap();
    let row = row_of(&app, user, "security.email_changed").await;

    // then
    assert!(row.params.values().all(|value| !value.contains('@')));
}
