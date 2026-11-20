//! The tests of the runtime of the Git providers: the server starts with and without the GitHub
//! App, and the App gets its installation tokens and the token of a user from a fake provider.

use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use scylla_core::api::ServeOptions;
use scylla_core::infra::vcs::{AppClaims, ProviderError, TokenScope, VcsClients};
use scylla_testkit::{FakeProvider, TestApp, TEST_APP_PUBLIC_KEY};
use serde_json::json;
use sqlx::PgPool;
use tonic_health::pb::health_check_response::ServingStatus;

use crate::app;

/// Makes the clients of a server that calls the fake, with the test GitHub App.
fn clients(fake: &FakeProvider) -> VcsClients {
    scylla_core::infra::tls::install();
    VcsClients::new(&fake.options(true)).expect("clients")
}

/// A server with no GitHub App starts: the integration needs no setting.
#[sqlx::test(migrations = false)]
async fn serve_starts_without_the_github_app(pool: PgPool) {
    // given
    let app = app(pool).await;

    // when
    let status = app.health_check().await.unwrap();

    // then
    assert_eq!(status, ServingStatus::Serving);
}

/// A server with the settings of a GitHub App starts.
#[sqlx::test(migrations = false)]
async fn serve_starts_with_the_github_app(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let options = ServeOptions {
        vcs: fake.options(true),
        ..ServeOptions::default()
    };
    let app = TestApp::spawn_with(pool, scylla_ce::edition(), options).await;

    // when
    let status = app.health_check().await.unwrap();

    // then
    assert_eq!(status, ServingStatus::Serving);
}

/// An installation token is narrowed to its repositories and permissions, asked with the JWT of
/// the App, and served again from the cache for the same scope.
#[tokio::test]
async fn installation_token_is_narrowed_and_cached() {
    // given
    let fake = FakeProvider::start().await;
    let path = "/app/installations/7/access_tokens";
    let token = json!({ "token": "ghs_test", "expires_at": "2099-01-01T00:00:00Z" });
    fake.respond("POST", path, 201, token, &[]);
    let clients = clients(&fake);
    let app = clients.github_app.as_ref().expect("the App");
    let narrow = TokenScope::default()
        .repository(42)
        .permission("contents", "read");

    // when
    let first = app.installation_token(7, &narrow).await.expect("token");
    let again = app.installation_token(7, &narrow).await.expect("token");
    let wider = app.installation_token(7, &TokenScope::default()).await;

    // then
    assert_eq!(first.token.expose(), "ghs_test");
    assert_eq!(again.token.expose(), "ghs_test");
    assert!(wider.is_ok());
    let requests = fake.requests_to("POST", path);
    assert_eq!(requests.len(), 2);
    let narrowed = json!({ "repository_ids": [42], "permissions": { "contents": "read" } });
    assert_eq!(requests[0].body, narrowed);
    assert_eq!(requests[1].body, json!({}));
    assert_eq!(
        requests[0].header("x-github-api-version"),
        Some("2022-11-28")
    );
    assert_eq!(
        requests[0].header("accept"),
        Some("application/vnd.github+json")
    );
    assert!(requests[0]
        .header("user-agent")
        .is_some_and(|agent| agent.starts_with("scylla/")));
    let jwt = requests[0]
        .header("authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .expect("the JWT of the App");
    let key = DecodingKey::from_rsa_pem(TEST_APP_PUBLIC_KEY.as_bytes()).expect("public key");
    let mut validation = Validation::new(Algorithm::RS256);
    validation.validate_aud = false;
    let claims = jsonwebtoken::decode::<AppClaims>(jwt, &key, &validation)
        .expect("a JWT signed by the key of the App")
        .claims;
    assert_eq!(claims.iss, "Iv1.test");
}

/// The code of a user gives its token, and a refused code gives the error of GitHub.
#[tokio::test]
async fn exchange_code_gives_the_token_of_the_user() {
    // given
    let fake = FakeProvider::start().await;
    let path = "/login/oauth/access_token";
    let token = json!({ "access_token": "ghu_test", "token_type": "bearer" });
    fake.respond("POST", path, 200, token, &[]);
    let refused = json!({ "error": "bad_verification_code" });
    fake.respond("POST", path, 200, refused, &[]);
    let clients = clients(&fake);
    let app = clients.github_app.as_ref().expect("the App");

    // when
    let good = app.exchange_code("good-code").await;
    let bad = app.exchange_code("bad-code").await;

    // then
    assert_eq!(good.expect("a token").expose(), "ghu_test");
    assert_eq!(
        bad.expect_err("a refusal"),
        ProviderError::Invalid("bad_verification_code".to_owned())
    );
    let requests = fake.requests_to("POST", path);
    assert_eq!(requests[0].header("accept"), Some("application/json"));
    assert_eq!(
        requests[0].body,
        json!({
            "client_id": "Iv1.test",
            "client_secret": "test-client-secret",
            "code": "good-code",
        })
    );
}
