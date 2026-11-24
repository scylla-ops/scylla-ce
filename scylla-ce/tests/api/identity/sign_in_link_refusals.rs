//! The tests of what the link with a password refuses: a wrong password with its lockout, an
//! expired link, another browser, an account with no password, an unverified email, and an account
//! that has an identity of the provider.

use scylla_proto::accounts::v1::account_service_client::AccountServiceClient;
use scylla_proto::accounts::v1::GetPendingLinkRequest;
use scylla_proto::auth::v1::token_service_client::TokenServiceClient;
use scylla_proto::auth::v1::ExchangeIdTokenRequest;
use sqlx::PgPool;
use tonic::{Code, Request};

use super::sign_in_link::{app_with_idp, identities, link, pending, APP_URL, EMAIL};
use crate::assert_code;
use crate::identity::accounts::{assert_refused, PASSWORD};
use crate::identity::browser::Browser;
use crate::identity::idp::{person, CLIENT_ID};

/// The failures before a lockout, as for `SignIn`.
const FAILURES: usize = 5;

/// Returns the address of the page for a code of the provider `idp`.
fn page(code: &str) -> String {
    format!("{APP_URL}/sign-in-error?code={code}&provider=idp")
}

/// Returns the number of links that wait.
async fn waiting(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM pending_links")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A wrong password links nothing, keeps the link, and counts for the lockout of `SignIn`. While the
/// account is locked, the right password is refused here and at `SignIn`.
#[sqlx::test(migrations = false)]
async fn a_wrong_password_counts_for_the_lockout(pool: PgPool) {
    // given
    let (app, _idp) = app_with_idp(pool.clone()).await;
    app.user_with(EMAIL, Some(PASSWORD)).await;
    let mut browser = Browser::new();
    browser.sign_in(&app, "idp", "").await;

    // when
    let first = link(&app, &browser, "not the password at all").await;
    let attempts: i32 = sqlx::query_scalar("SELECT failed_attempts FROM passwords")
        .fetch_one(&pool)
        .await
        .unwrap();
    let still = pending(&app, &browser).await;
    for _ in 1..FAILURES {
        let wrong = link(&app, &browser, "not the password at all").await;
        assert_refused(wrong, "WRONG_PASSWORD");
    }
    let locked: bool = sqlx::query_scalar("SELECT locked_until > now() FROM passwords")
        .fetch_one(&pool)
        .await
        .unwrap();
    let right = link(&app, &browser, PASSWORD).await;
    let sign_in = app.sign_in(EMAIL, PASSWORD).await;

    // then
    assert_refused(first, "WRONG_PASSWORD");
    assert_eq!(attempts, 1);
    assert!(still.is_ok());
    assert!(locked);
    assert_refused(right, "WRONG_PASSWORD");
    assert_eq!(sign_in.unwrap_err().code(), Code::Unauthenticated);
    assert_eq!(identities(&pool).await, 0);
}

/// An expired link finds nothing, also with the right password.
#[sqlx::test(migrations = false)]
async fn an_expired_link_finds_nothing(pool: PgPool) {
    // given
    let (app, _idp) = app_with_idp(pool.clone()).await;
    app.user_with(EMAIL, Some(PASSWORD)).await;
    let mut browser = Browser::new();
    browser.sign_in(&app, "idp", "").await;
    sqlx::query("UPDATE pending_links SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();

    // when
    let read = pending(&app, &browser).await;
    let linked = link(&app, &browser, PASSWORD).await;

    // then
    assert_eq!(read.unwrap_err().code(), Code::NotFound);
    assert_eq!(linked.unwrap_err().code(), Code::NotFound);
    assert_eq!(identities(&pool).await, 0);
}

/// The link needs the cookie of the browser of the callback, and an allowed web origin.
#[sqlx::test(migrations = false)]
async fn a_link_needs_its_browser_and_a_web_origin(pool: PgPool) {
    // given
    let (app, _idp) = app_with_idp(pool.clone()).await;
    app.user_with(EMAIL, Some(PASSWORD)).await;
    let mut browser = Browser::new();
    browser.sign_in(&app, "idp", "").await;
    let cookie = format!("scylla_pending={}", browser.cookies["scylla_pending"]);

    // when
    let other = link(&app, &Browser::new(), PASSWORD).await;
    let mut other_token = Browser::new();
    other_token
        .cookies
        .insert("scylla_pending".to_owned(), "made-up".to_owned());
    let made_up = link(&app, &other_token, PASSWORD).await;
    let mut no_origin = Request::new(GetPendingLinkRequest {});
    no_origin
        .metadata_mut()
        .insert("cookie", cookie.parse().unwrap());
    let mut other_site = Request::new(GetPendingLinkRequest {});
    other_site
        .metadata_mut()
        .insert("cookie", cookie.parse().unwrap());
    other_site
        .metadata_mut()
        .insert("origin", "https://evil.example".parse().unwrap());
    let mut client = AccountServiceClient::new(app.channel.clone());
    let without = client.get_pending_link(no_origin).await;
    let from_other_site = client.get_pending_link(other_site).await;

    // then
    assert_eq!(other.unwrap_err().code(), Code::NotFound);
    assert_eq!(made_up.unwrap_err().code(), Code::NotFound);
    assert_eq!(without.unwrap_err().code(), Code::PermissionDenied);
    assert_eq!(from_other_site.unwrap_err().code(), Code::PermissionDenied);
    assert_eq!(identities(&pool).await, 0);
}

/// An account with no password stays refused with the page of the error, and no link waits.
#[sqlx::test(migrations = false)]
async fn an_account_with_no_password_stays_refused(pool: PgPool) {
    // given
    let (app, _idp) = app_with_idp(pool.clone()).await;
    app.user_with(EMAIL, None).await;
    let mut browser = Browser::new();

    // when
    let step = browser.sign_in(&app, "idp", "").await;

    // then
    assert_eq!(step.location, page("email_in_use"));
    assert_eq!(waiting(&pool).await, 0);
    assert!(!browser.cookies.contains_key("scylla_pending"));
}

/// An email that the provider did not verify never matches an account.
#[sqlx::test(migrations = false)]
async fn an_unverified_email_matches_no_account(pool: PgPool) {
    // given
    let (app, idp) = app_with_idp(pool.clone()).await;
    app.user_with(EMAIL, Some(PASSWORD)).await;
    idp.sign_in_as(person("alice", EMAIL, false));
    let mut browser = Browser::new();

    // when
    let step = browser.sign_in(&app, "idp", "").await;

    // then
    assert_eq!(step.location, page("email_unverified"));
    assert_eq!(waiting(&pool).await, 0);
    assert!(!browser.cookies.contains_key("scylla_pending"));
}

/// An account that has an identity of the provider keeps it: another subject with the same email can
/// be another person. This holds at the callback and when the identity comes after the callback.
#[sqlx::test(migrations = false)]
async fn an_account_keeps_its_identity_of_the_provider(pool: PgPool) {
    // given
    let (app, idp) = app_with_idp(pool.clone()).await;
    app.user_with(EMAIL, Some(PASSWORD)).await;
    let session = app.sign_in(EMAIL, PASSWORD).await.unwrap();
    let mut signed_in = Browser::new();
    signed_in
        .cookies
        .insert("scylla_session".to_owned(), session.refresh_token);

    // when
    idp.sign_in_as(person("second", EMAIL, true));
    let mut waits = Browser::new();
    let first_step = waits.sign_in(&app, "idp", "").await;
    idp.sign_in_as(person("alice", EMAIL, true));
    signed_in.sign_in(&app, "idp", "link=true").await;
    let late = link(&app, &waits, PASSWORD).await;
    idp.sign_in_as(person("third", EMAIL, true));
    let at_callback = Browser::new().sign_in(&app, "idp", "").await;

    // then
    assert_eq!(first_step.location, format!("{APP_URL}/sign-in-link"));
    assert_refused(late, "ALREADY_LINKED");
    assert_eq!(at_callback.location, page("already_linked"));
    assert_eq!(identities(&pool).await, 1);
}

/// An identity that another account took after the callback stays with that account.
#[sqlx::test(migrations = false)]
async fn an_identity_of_another_account_stays_with_it(pool: PgPool) {
    // given
    let (app, _idp) = app_with_idp(pool.clone()).await;
    app.user_with(EMAIL, Some(PASSWORD)).await;
    app.user_with("bob@example.com", Some(PASSWORD)).await;
    let session = app.sign_in("bob@example.com", PASSWORD).await.unwrap();
    let mut bob = Browser::new();
    bob.cookies
        .insert("scylla_session".to_owned(), session.refresh_token);
    let mut waits = Browser::new();
    waits.sign_in(&app, "idp", "").await;

    // when
    bob.sign_in(&app, "idp", "link=true").await;
    let late = link(&app, &waits, PASSWORD).await;

    // then
    assert_refused(late, "IDENTITY_IN_USE");
    assert_eq!(identities(&pool).await, 1);
}

/// An account whose email nobody proved links nothing: someone else can have made it first, and
/// keep its password.
#[sqlx::test(migrations = false)]
async fn an_account_with_an_email_that_nobody_proved_links_nothing(pool: PgPool) {
    // given
    let (app, _idp) = app_with_idp(pool.clone()).await;
    app.sign_up(EMAIL, PASSWORD, "").await.unwrap();
    let mut browser = Browser::new();

    // when
    browser.sign_in(&app, "idp", "").await;
    let not_proved = link(&app, &browser, PASSWORD).await;

    // then
    assert_refused(not_proved, "EMAIL_NOT_VERIFIED");
    assert_eq!(identities(&pool).await, 0);
}

/// A disabled account gets its own page at the callback, and no link waits.
#[sqlx::test(migrations = false)]
async fn a_disabled_account_gets_its_page(pool: PgPool) {
    // given
    let (app, _idp) = app_with_idp(pool.clone()).await;
    let admin = app.admin().await;
    let user = app.user_with(EMAIL, Some(PASSWORD)).await;
    app.set_user_status(admin, &user.to_string(), "disabled")
        .await
        .unwrap();

    // when
    let step = Browser::new().sign_in(&app, "idp", "").await;

    // then
    assert_eq!(step.location, page("account_inactive"));
    assert_eq!(waiting(&pool).await, 0);
}

/// A native client has no page for a password, so a known email stays refused.
#[sqlx::test(migrations = false)]
async fn a_native_client_keeps_the_refusal(pool: PgPool) {
    // given
    let (app, idp) = app_with_idp(pool.clone()).await;
    app.user_with(EMAIL, Some(PASSWORD)).await;
    let id_token = idp.id_token(&person("alice", EMAIL, true), CLIENT_ID, 60);

    // when
    let exchanged = TokenServiceClient::new(app.channel.clone())
        .exchange_id_token(ExchangeIdTokenRequest { id_token })
        .await;

    // then
    assert_code(exchanged, Code::FailedPrecondition);
    assert_eq!(waiting(&pool).await, 0);
}
