//! The tests of the sessions: the refresh with rotation, the sign-out, the list and the revoke,
//! a disabled user, and the cookie of a browser.

use scylla_core::api::ServeOptions;
use scylla_proto::accounts::v1::account_service_client::AccountServiceClient;
use scylla_proto::accounts::v1::SignInRequest;
use scylla_proto::auth::v1::token_service_client::TokenServiceClient;
use scylla_proto::auth::v1::RefreshRequest;
use scylla_testkit::{text, TestApp};
use sqlx::PgPool;
use tonic::metadata::MetadataValue;
use tonic::{Code, Request};

use crate::identity::accounts::PASSWORD;
use crate::{app, assert_code};

/// The web origin of the tests.
const ORIGIN: &str = "https://app.example";

/// A refresh gives a new token and a new refresh token. The old one stops, and its second use
/// closes the session.
#[sqlx::test(migrations = false)]
async fn refresh_rotates_and_detects_a_reuse(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "eve@example.com";
    app.user_with(email, Some(PASSWORD)).await;
    let session = app.sign_in(email, PASSWORD).await.unwrap();

    // when
    let first = app.refresh(&session.refresh_token).await.unwrap();
    let reuse = app.refresh(&session.refresh_token).await;
    let after = app.refresh(&first.refresh_token).await;

    // then
    assert_ne!(first.refresh_token, session.refresh_token);
    assert_code(reuse, Code::Unauthenticated);
    assert_code(after, Code::Unauthenticated);
}

/// A sign-out stops the tokens of its session at once.
#[sqlx::test(migrations = false)]
async fn sign_out_stops_the_tokens(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "fay@example.com";
    app.user_with(email, Some(PASSWORD)).await;
    let session = app.sign_in(email, PASSWORD).await.unwrap();
    let token = session.token.unwrap().value;

    // when
    let before = app.get_me(&token).await;
    app.sign_out(&session.refresh_token).await.unwrap();
    let after = app.get_me(&token).await;

    // then
    assert!(before.is_ok(), "{before:?}");
    assert_code(after, Code::Unauthenticated);
    assert_code(
        app.refresh(&session.refresh_token).await,
        Code::Unauthenticated,
    );
}

/// The list shows the open sessions with the current one, and a revoke stops one session.
#[sqlx::test(migrations = false)]
async fn a_user_lists_and_revokes_its_sessions(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "gus@example.com";
    app.user_with(email, Some(PASSWORD)).await;
    let this = app.sign_in(email, PASSWORD).await.unwrap();
    let other = app.sign_in(email, PASSWORD).await.unwrap();
    let token = this.token.unwrap().value;
    let other_token = other.token.unwrap().value;

    // when
    let before = app.list_sessions(&token).await.unwrap();
    let other_id = before.iter().find(|session| !session.current).unwrap();
    app.revoke_session(&token, text(&other_id.id))
        .await
        .unwrap();
    let after = app.list_sessions(&token).await.unwrap();
    let unknown = app.revoke_session(&token, text(&other_id.id)).await;

    // then
    assert_eq!(before.len(), 2);
    assert_eq!(after.len(), 1);
    assert!(after[0].current && after[0].methods == ["password"]);
    assert_code(app.get_me(&other_token).await, Code::Unauthenticated);
    assert_code(unknown, Code::NotFound);
}

/// A disabled user stops at once: its tokens and its refresh tokens.
#[sqlx::test(migrations = false)]
async fn a_disabled_user_stops(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "hal@example.com";
    let user = app.user_with(email, Some(PASSWORD)).await;
    let session = app.sign_in(email, PASSWORD).await.unwrap();

    // when
    sqlx::query!(
        "UPDATE users SET status = 'disabled' WHERE id = $1",
        user.to_db()
    )
    .execute(&app.pool)
    .await
    .unwrap();

    // then
    let token = session.token.unwrap().value;
    assert_code(app.get_me(&token).await, Code::Unauthenticated);
    assert_code(
        app.refresh(&session.refresh_token).await,
        Code::Unauthenticated,
    );
    assert_code(app.sign_in(email, PASSWORD).await, Code::Unauthenticated);
}

/// A browser keeps the refresh token in an HttpOnly cookie. A refresh with the cookie needs an
/// allowed web origin.
#[sqlx::test(migrations = false)]
async fn a_browser_refreshes_with_its_cookie(pool: PgPool) {
    // given
    let mut options = ServeOptions::default();
    options.accounts.web_origins = vec![ORIGIN.to_owned()];
    let app = TestApp::spawn_with(pool, scylla_ce::edition(), options).await;
    let email = "ida@example.com";
    app.user_with(email, Some(PASSWORD)).await;
    let request = SignInRequest {
        email: email.to_owned(),
        password: PASSWORD.to_owned(),
        cookie: true,
    };
    let signed_in = AccountServiceClient::new(app.channel.clone())
        .sign_in(request)
        .await
        .unwrap();
    let set_cookie = signed_in
        .metadata()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let cookie = set_cookie.split(';').next().unwrap().to_owned();

    // when
    let refresh = |origin: Option<&str>| {
        let mut request = Request::new(RefreshRequest::default());
        let metadata = request.metadata_mut();
        metadata.insert("cookie", MetadataValue::try_from(cookie.as_str()).unwrap());
        if let Some(origin) = origin {
            metadata.insert("origin", MetadataValue::try_from(origin).unwrap());
        }
        request
    };
    let mut client = TokenServiceClient::new(app.channel.clone());
    let without_origin = client.refresh(refresh(None)).await;
    let other_origin = client.refresh(refresh(Some("https://evil.example"))).await;
    let refreshed = client.refresh(refresh(Some(ORIGIN))).await.unwrap();

    // then
    assert!(set_cookie.contains("HttpOnly") && cookie.starts_with("scylla_session="));
    assert!(signed_in.into_inner().refresh_token.is_empty());
    assert_code(without_origin, Code::PermissionDenied);
    assert_code(other_origin, Code::PermissionDenied);
    assert!(refreshed.metadata().get("set-cookie").is_some());
    assert!(refreshed.into_inner().refresh_token.is_empty());
}
