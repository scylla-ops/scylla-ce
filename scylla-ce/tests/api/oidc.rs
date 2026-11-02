//! The tests of the OIDC sign-in, with a local issuer that serves its keys.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::routing::get;
use axum::{Json, Router};
use biscuit_auth::KeyPair;
use jsonwebtoken::jwk::{Jwk, JwkSet};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use scylla_core::features::users::OidcVerifier;
use scylla_core::server::ServeOptions;
use scylla_proto::auth::v1::token_service_client::TokenServiceClient;
use scylla_proto::auth::v1::ExchangeIdTokenRequest;
use scylla_testkit::{text, with_token, TestApp};
use serde::Serialize;
use sqlx::PgPool;
use tokio::net::TcpListener;
use tonic::{Code, Status};

use crate::{app, assert_code};

/// The audience of the tests.
const AUDIENCE: &str = "scylla";

/// The key id of the issuer of the tests.
const KID: &str = "test-key";

/// A local OIDC issuer: its URL and its signing key.
struct Issuer {
    /// The issuer URL.
    url: String,
    /// The key that signs the ID tokens.
    key: EncodingKey,
}

/// The discovery document.
#[derive(Clone, Serialize)]
struct Discovery {
    /// The issuer.
    issuer: String,
    /// The URL of the keys.
    jwks_uri: String,
}

/// The claims of an ID token.
#[derive(Serialize)]
struct Claims {
    /// The issuer.
    iss: String,
    /// The audience.
    aud: String,
    /// The subject.
    sub: String,
    /// The expiry.
    exp: u64,
}

/// The PKCS#8 v1 prefix of an Ed25519 private key, before its 32-byte seed.
const PKCS8_ED25519: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

/// Starts an issuer that serves its discovery document and its Ed25519 key.
async fn issuer() -> Issuer {
    let seed = KeyPair::new().private().to_bytes();
    let key = EncodingKey::from_ed_der(&[&PKCS8_ED25519[..], &seed[..]].concat());
    let mut jwk = Jwk::from_encoding_key(&key, Algorithm::EdDSA).unwrap();
    jwk.common.key_id = Some(KID.to_owned());
    let keys = JwkSet { keys: vec![jwk] };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let discovery = Discovery {
        issuer: url.clone(),
        jwks_uri: format!("{url}/jwks"),
    };
    let router = Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(move || async move { Json(discovery) }),
        )
        .route("/jwks", get(move || async move { Json(keys) }));
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Issuer { url, key }
}

impl Issuer {
    /// Signs an ID token for a subject.
    fn id_token(&self, subject: &str, audience: &str, seconds: i64) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = Claims {
            iss: self.url.clone(),
            aud: audience.to_owned(),
            sub: subject.to_owned(),
            exp: now.saturating_add_signed(seconds),
        };
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(KID.to_owned());
        encode(&header, &claims, &self.key).unwrap()
    }
}

/// Starts the Community edition with the issuer of the tests.
async fn app_with(pool: PgPool, issuer: &Issuer) -> TestApp {
    let verifier = OidcVerifier::discover(&issuer.url, AUDIENCE).await.unwrap();
    let options = ServeOptions {
        oidc: Some(Arc::new(verifier)),
        ..ServeOptions::default()
    };
    TestApp::spawn_with(pool, scylla_ce::edition(), options).await
}

/// Exchanges an ID token for a token.
async fn exchange(app: &TestApp, id_token: String) -> Result<String, Status> {
    let response = TokenServiceClient::new(app.channel.clone())
        .exchange_id_token(ExchangeIdTokenRequest { id_token })
        .await?;
    Ok(response.into_inner().token.unwrap().value)
}

/// The author of a project that a token creates.
async fn author(app: &TestApp, token: &str) -> String {
    let request = scylla_proto::projects::v1::CreateProjectRequest {
        owner_id: None,
        name: "alpha".to_owned(),
    };
    let response = app
        .projects
        .clone()
        .create_project(with_token(token, request))
        .await
        .unwrap();
    text(&response.into_inner().project.unwrap().created_by).to_owned()
}

/// The first sign-in creates the user, and the next one finds the same user.
#[sqlx::test(migrations = false)]
async fn exchange_id_token_signs_in_the_same_user(pool: PgPool) {
    // given
    let issuer = issuer().await;
    let app = app_with(pool, &issuer).await;

    // when
    let first = exchange(&app, issuer.id_token("alice", AUDIENCE, 60))
        .await
        .unwrap();
    let second = exchange(&app, issuer.id_token("alice", AUDIENCE, 60))
        .await
        .unwrap();

    // then
    assert_eq!(author(&app, &first).await, author(&app, &second).await);
}

/// A token for another audience, or an expired one, is refused.
#[sqlx::test(migrations = false)]
async fn exchange_id_token_refuses_a_bad_id_token(pool: PgPool) {
    // given
    let issuer = issuer().await;
    let app = app_with(pool, &issuer).await;

    // when
    let other_audience = exchange(&app, issuer.id_token("alice", "other", 60)).await;
    let expired = exchange(&app, issuer.id_token("alice", AUDIENCE, -3600)).await;
    let garbage = exchange(&app, "nope".to_owned()).await;

    // then
    assert_code(other_audience, Code::Unauthenticated);
    assert_code(expired, Code::Unauthenticated);
    assert_code(garbage, Code::Unauthenticated);
}

/// With no issuer in the settings, no ID token is accepted.
#[sqlx::test(migrations = false)]
async fn exchange_id_token_needs_an_issuer(pool: PgPool) {
    // given
    let issuer = issuer().await;
    let app = app(pool).await;

    // when
    let result = exchange(&app, issuer.id_token("alice", AUDIENCE, 60)).await;

    // then
    assert_code(result, Code::Unauthenticated);
}
