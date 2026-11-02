//! A fake identity provider for the tests, with OIDC and GitHub endpoints. It accepts each
//! sign-in at once, for the person that the test sets.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use biscuit_auth::KeyPair;
use jsonwebtoken::jwk::{Jwk, JwkSet};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

/// The client id of Scylla at the fake provider.
pub(crate) const CLIENT_ID: &str = "scylla";

/// The key id of the fake provider.
const KID: &str = "test-key";

/// The PKCS#8 v1 prefix of an Ed25519 private key, before its 32-byte seed.
const PKCS8_ED25519: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

/// The person that signs in at the fake provider.
#[derive(Clone, Debug)]
pub(crate) struct Person {
    /// The id of the person.
    pub(crate) sub: String,
    /// The email.
    pub(crate) email: String,
    /// `true` if the provider verified the email.
    pub(crate) email_verified: bool,
}

/// A code that waits for its exchange.
struct Grant {
    /// The nonce of the sign-in.
    nonce: Option<String>,
    /// The PKCE challenge.
    challenge: String,
    /// The person.
    person: Person,
}

/// What the provider keeps.
struct Inner {
    /// The URL of the provider.
    url: String,
    /// The signing key.
    key: EncodingKey,
    /// The public keys.
    keys: JwkSet,
    /// The person of the next sign-in.
    person: Mutex<Person>,
    /// The codes, by their value.
    grants: Mutex<HashMap<String, Grant>>,
}

/// A fake identity provider on a free port.
#[derive(Clone)]
pub(crate) struct Idp(Arc<Inner>);

impl Idp {
    /// Starts a provider, with a first person.
    pub(crate) async fn start() -> Self {
        let seed = KeyPair::new().private().to_bytes();
        let key = EncodingKey::from_ed_der(&[&PKCS8_ED25519[..], &seed[..]].concat());
        let mut jwk = Jwk::from_encoding_key(&key, Algorithm::EdDSA).unwrap();
        jwk.common.key_id = Some(KID.to_owned());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let person = person("alice", "alice@idp.example", true);
        let idp = Self(Arc::new(Inner {
            url,
            key,
            keys: JwkSet { keys: vec![jwk] },
            person: Mutex::new(person),
            grants: Mutex::new(HashMap::new()),
        }));
        let router = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks))
            .route("/authorize", get(authorize))
            .route("/token", post(token))
            .route("/login/oauth/authorize", get(authorize))
            .route("/login/oauth/access_token", post(github_token))
            .route("/user", get(github_user))
            .route("/user/emails", get(github_emails))
            .with_state(idp.clone());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        idp
    }

    /// Returns the URL of the provider, its issuer.
    pub(crate) fn url(&self) -> &str {
        &self.0.url
    }

    /// Sets the person of the next sign-in.
    pub(crate) fn sign_in_as(&self, person: Person) {
        *self.0.person.lock().unwrap() = person;
    }

    /// Signs an ID token for a person and an audience, valid `seconds`.
    pub(crate) fn id_token(&self, person: &Person, audience: &str, seconds: i64) -> String {
        self.sign(person, audience, seconds, None)
    }

    /// Signs an ID token.
    fn sign(&self, person: &Person, audience: &str, seconds: i64, nonce: Option<&str>) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut claims = json!({
            "iss": self.0.url,
            "aud": audience,
            "sub": person.sub,
            "exp": now.saturating_add_signed(seconds),
            "email": person.email,
            "email_verified": person.email_verified,
            "name": "Alice Example",
        });
        if let Some(nonce) = nonce {
            claims["nonce"] = json!(nonce);
        }
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(KID.to_owned());
        encode(&header, &claims, &self.0.key).unwrap()
    }
}

/// Makes a person.
pub(crate) fn person(sub: &str, email: &str, email_verified: bool) -> Person {
    Person {
        sub: sub.to_owned(),
        email: email.to_owned(),
        email_verified,
    }
}

/// Serves the discovery document.
async fn discovery(State(idp): State<Idp>) -> Json<Value> {
    let url = &idp.0.url;
    Json(json!({
        "issuer": url,
        "authorization_endpoint": format!("{url}/authorize"),
        "token_endpoint": format!("{url}/token"),
        "jwks_uri": format!("{url}/jwks"),
    }))
}

/// Serves the public keys.
async fn jwks(State(idp): State<Idp>) -> Json<JwkSet> {
    Json(idp.0.keys.clone())
}

/// Accepts each sign-in at once: keeps a code for the person, and goes back to the callback.
async fn authorize(
    State(idp): State<Idp>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let code = format!("code-{}", idp.0.grants.lock().unwrap().len());
    let grant = Grant {
        nonce: query.get("nonce").cloned(),
        challenge: query["code_challenge"].clone(),
        person: idp.0.person.lock().unwrap().clone(),
    };
    idp.0.grants.lock().unwrap().insert(code.clone(), grant);
    let location = format!(
        "{}?code={code}&state={}",
        query["redirect_uri"], query["state"]
    );
    (StatusCode::SEE_OTHER, [(header::LOCATION, location)]).into_response()
}

/// Takes the grant of a code if the PKCE verifier matches.
fn redeem(idp: &Idp, form: &HashMap<String, String>) -> Option<Grant> {
    let grant = idp.0.grants.lock().unwrap().remove(&form["code"])?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(form["code_verifier"].as_bytes()));
    (challenge == grant.challenge).then_some(grant)
}

/// Exchanges an OIDC code for an ID token.
async fn token(State(idp): State<Idp>, Form(form): Form<HashMap<String, String>>) -> Response {
    let Some(grant) = redeem(&idp, &form) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let id_token = idp.sign(
        &grant.person,
        &form["client_id"],
        60,
        grant.nonce.as_deref(),
    );
    Json(json!({ "id_token": id_token, "token_type": "Bearer" })).into_response()
}

/// Exchanges a GitHub code for an access token: the subject of the person.
async fn github_token(
    State(idp): State<Idp>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    match redeem(&idp, &form) {
        Some(grant) => Json(json!({ "access_token": grant.person.sub })).into_response(),
        None => StatusCode::BAD_REQUEST.into_response(),
    }
}

/// Serves the GitHub user of the access token.
async fn github_user(headers: axum::http::HeaderMap) -> Json<Value> {
    let sub = bearer(&headers);
    Json(json!({ "id": sub.len() * 1000 + 7, "login": sub, "name": null }))
}

/// Serves the emails of the GitHub user: the person of the provider.
async fn github_emails(State(idp): State<Idp>) -> Json<Value> {
    let person = idp.0.person.lock().unwrap().clone();
    Json(json!([{ "email": person.email, "primary": true, "verified": person.email_verified }]))
}

/// Reads the Bearer token of a request.
fn bearer(headers: &axum::http::HeaderMap) -> String {
    let value = headers[header::AUTHORIZATION].to_str().unwrap();
    value.trim_start_matches("Bearer ").to_owned()
}
