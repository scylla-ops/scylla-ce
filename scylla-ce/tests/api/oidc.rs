//! The tests of `ExchangeIdToken`: a native client gives an ID token of an identity provider of
//! the server, and gets a token and a session.

use scylla_core::server::ServeOptions;
use scylla_core::SignUpMode;
use scylla_proto::auth::v1::token_service_client::TokenServiceClient;
use scylla_proto::auth::v1::{ExchangeIdTokenRequest, ExchangeIdTokenResponse};
use scylla_testkit::TestApp;
use sqlx::PgPool;
use tonic::{Code, Status};

use crate::assert_code;
use crate::idp::{person, Idp, CLIENT_ID};

/// Starts the Community edition with open sign-up and the provider `idp`.
async fn app_with(pool: PgPool, idp: &Idp, allow_sign_up: bool) -> TestApp {
    let mut options = ServeOptions::default();
    options.accounts.sign_up_mode = SignUpMode::Open;
    let app = TestApp::spawn_with(pool, scylla_ce::edition(), options).await;
    app.add_oidc_provider("idp", idp.url(), CLIENT_ID, None, allow_sign_up)
        .await;
    app
}

/// Exchanges an ID token.
async fn exchange(app: &TestApp, id_token: String) -> Result<ExchangeIdTokenResponse, Status> {
    let response = TokenServiceClient::new(app.channel.clone())
        .exchange_id_token(ExchangeIdTokenRequest { id_token })
        .await?;
    Ok(response.into_inner())
}

/// The first exchange makes the user with the verified email of the provider, and the next one
/// finds the same user. Each exchange opens a session.
#[sqlx::test(migrations = false)]
async fn exchange_id_token_signs_in_the_same_user(pool: PgPool) {
    // given
    let idp = Idp::start().await;
    let app = app_with(pool, &idp, true).await;
    let alice = person("alice", "alice@idp.example", true);

    // when
    let first = exchange(&app, idp.id_token(&alice, CLIENT_ID, 60))
        .await
        .unwrap();
    let second = exchange(&app, idp.id_token(&alice, CLIENT_ID, 60))
        .await
        .unwrap();

    // then
    let me = app.get_me(&first.token.unwrap().value).await.unwrap();
    let again = app.get_me(&second.token.unwrap().value).await.unwrap();
    assert_eq!(me.id, again.id);
    assert_eq!(me.email, "alice@idp.example");
    assert!(!first.refresh_token.is_empty() && first.refresh_token != second.refresh_token);
}

/// A token for another audience, an expired one, and a text that is not a token are refused.
#[sqlx::test(migrations = false)]
async fn exchange_id_token_refuses_a_bad_id_token(pool: PgPool) {
    // given
    let idp = Idp::start().await;
    let app = app_with(pool, &idp, true).await;
    let alice = person("alice", "alice@idp.example", true);

    // when
    let other_audience = exchange(&app, idp.id_token(&alice, "other", 60)).await;
    let expired = exchange(&app, idp.id_token(&alice, CLIENT_ID, -3600)).await;
    let garbage = exchange(&app, "nope".to_owned()).await;

    // then
    assert_code(other_audience, Code::Unauthenticated);
    assert_code(expired, Code::Unauthenticated);
    assert_code(garbage, Code::Unauthenticated);
}

/// An issuer that is not a provider of the server is refused.
#[sqlx::test(migrations = false)]
async fn exchange_id_token_needs_a_provider_of_the_issuer(pool: PgPool) {
    // given
    let idp = Idp::start().await;
    let app = TestApp::spawn(pool, scylla_ce::edition()).await;
    let alice = person("alice", "alice@idp.example", true);

    // when
    let result = exchange(&app, idp.id_token(&alice, CLIENT_ID, 60)).await;

    // then
    assert_code(result, Code::Unauthenticated);
}

/// A provider with no sign-up makes no user, and an unverified email makes no user.
#[sqlx::test(migrations = false)]
async fn exchange_id_token_makes_a_user_only_as_allowed(pool: PgPool) {
    // given
    let idp = Idp::start().await;
    let closed = app_with(pool, &idp, false).await;
    let unverified = person("bob", "bob@idp.example", false);
    let alice = person("alice", "alice@idp.example", true);
    closed
        .add_oidc_provider("other", idp.url(), CLIENT_ID, None, true)
        .await;

    // when
    let no_sign_up = exchange(&closed, idp.id_token(&alice, CLIENT_ID, 60)).await;
    let no_email = exchange(&closed, idp.id_token(&unverified, CLIENT_ID, 60)).await;

    // then
    assert_code(no_sign_up, Code::FailedPrecondition);
    assert_code(no_email, Code::FailedPrecondition);
}
