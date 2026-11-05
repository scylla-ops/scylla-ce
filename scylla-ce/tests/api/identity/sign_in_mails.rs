//! The tests of the mails of the ways to sign in: each new link and each unlink sends a mail to the
//! owner of the account, and a mail that fails does not stop the link or the unlink.

use std::time::Duration;

use scylla_testkit::TestApp;
use sqlx::PgPool;

use super::sign_in_link::{app_with_idp, identities, link, EMAIL};
use crate::identity::accounts::PASSWORD;
use crate::identity::browser::Browser;

/// The time after which a mail that the server skips would have come.
const SETTLE: Duration = Duration::from_millis(300);

/// Returns a browser with the session of the account, and its access token.
async fn signed_in(app: &TestApp) -> (Browser, String) {
    let session = app.sign_in(EMAIL, PASSWORD).await.unwrap();
    let mut browser = Browser::new();
    browser
        .cookies
        .insert("scylla_session".to_owned(), session.refresh_token);
    (browser, session.token.unwrap().value)
}

/// A link from the page of the account sends a mail with the name of the provider. The same link
/// again adds nothing, so it sends no mail.
#[sqlx::test(migrations = false)]
async fn a_link_from_the_account_sends_a_mail(pool: PgPool) {
    // given
    let (app, _idp) = app_with_idp(pool.clone()).await;
    app.user_with(EMAIL, Some(PASSWORD)).await;
    let (mut browser, _) = signed_in(&app).await;

    // when
    let linked = browser.sign_in(&app, "idp", "link=true").await;
    let mails = app.mails.wait(EMAIL, 1).await;
    browser.sign_in(&app, "idp", "link=true").await;
    tokio::time::sleep(SETTLE).await;

    // then
    assert_eq!(linked.status, 303);
    assert!(mails[0].subject.contains("new sign-in method"));
    assert!(mails[0].subject.contains("idp"));
    assert!(mails[0]
        .body
        .contains("sign in to your Scylla account with idp"));
    assert_eq!(app.mails.to(EMAIL).len(), 1);
    assert_eq!(identities(&pool).await, 1);
}

/// An unlink sends a mail with the name of the provider, and an unlink that finds nothing sends
/// none.
#[sqlx::test(migrations = false)]
async fn an_unlink_sends_a_mail(pool: PgPool) {
    // given
    let (app, _idp) = app_with_idp(pool.clone()).await;
    app.user_with(EMAIL, Some(PASSWORD)).await;
    let (mut browser, token) = signed_in(&app).await;
    browser.sign_in(&app, "idp", "link=true").await;
    app.mails.wait(EMAIL, 1).await;

    // when
    let nothing = app.unlink_identity(&token, "idp", "nobody").await;
    tokio::time::sleep(SETTLE).await;
    let count_after_nothing = app.mails.to(EMAIL).len();
    app.unlink_identity(&token, "idp", "alice").await.unwrap();
    let mails = app.mails.wait(EMAIL, 2).await;

    // then
    assert!(nothing.is_err());
    assert_eq!(count_after_nothing, 1);
    assert!(mails[1].subject.contains("left your Scylla account"));
    assert!(mails[1].subject.contains("idp"));
    assert!(mails[1].body.contains("no longer sign in"));
    assert_eq!(identities(&pool).await, 0);
}

/// A refused unlink sends no mail: the last way to sign in stays.
#[sqlx::test(migrations = false)]
async fn a_refused_unlink_sends_no_mail(pool: PgPool) {
    // given
    let (app, _idp) = app_with_idp(pool.clone()).await;
    let mut browser = Browser::new();
    browser.sign_in(&app, "idp", "").await;
    let refresh = &browser.cookies["scylla_session"];
    let token = app.refresh(refresh).await.unwrap().token.unwrap().value;

    // when
    let refused = app.unlink_identity(&token, "idp", "alice").await;
    tokio::time::sleep(SETTLE).await;

    // then
    assert!(refused.is_err());
    assert!(app.mails.to(EMAIL).is_empty());
    assert_eq!(identities(&pool).await, 1);
}

/// A mail that fails does not stop a link from the account, an unlink, or a link with the password.
#[sqlx::test(migrations = false)]
async fn a_mail_that_fails_stops_nothing(pool: PgPool) {
    // given
    let (app, _idp) = app_with_idp(pool.clone()).await;
    app.user_with(EMAIL, Some(PASSWORD)).await;
    let (mut browser, token) = signed_in(&app).await;
    app.mails.fail_sends();

    // when
    let from_account = browser.sign_in(&app, "idp", "link=true").await;
    let after_link = identities(&pool).await;
    let unlinked = app.unlink_identity(&token, "idp", "alice").await;
    let after_unlink = identities(&pool).await;
    let mut waits = Browser::new();
    waits.sign_in(&app, "idp", "").await;
    let with_password = link(&app, &waits, PASSWORD).await;

    // then
    assert_eq!(from_account.status, 303);
    assert_eq!(after_link, 1);
    assert!(unlinked.is_ok(), "{unlinked:?}");
    assert_eq!(after_unlink, 0);
    assert!(with_password.is_ok(), "{with_password:?}");
    assert_eq!(identities(&pool).await, 1);
    assert!(app.mails.to(EMAIL).is_empty());
}
