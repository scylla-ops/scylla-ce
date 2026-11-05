//! The OIDC client of the identity providers: discovery, the authorization URL, the exchange of
//! a code, and the check of an ID token: issuer, audience, expiry, nonce, and a JWKS signature.

use std::collections::HashMap;
use std::sync::Arc;

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use scylla_extension::AccessError;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use url::Url;

use super::calls::{send, send_exchange, unavailable};
use super::ExternalIdentity;
use crate::Error;

/// The settings of an OIDC provider, in its JSON.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OidcSettings {
    /// The issuer, as in the `iss` claim.
    pub issuer: String,
    /// The client id of Scylla at the issuer, the audience of its ID tokens.
    pub client_id: String,
    /// The scopes to ask for.
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,
}

/// The scopes of a sign-in: the identity, the email and the name.
fn default_scopes() -> Vec<String> {
    ["openid", "email", "profile"].map(str::to_owned).to_vec()
}

/// The claims that the server reads.
#[derive(Deserialize)]
struct Claims {
    /// The subject.
    sub: String,
    /// The email.
    #[serde(default)]
    email: Option<String>,
    /// `true` if the provider verified the email.
    #[serde(default)]
    email_verified: Option<bool>,
    /// The name of the person.
    #[serde(default)]
    name: Option<String>,
    /// The nonce of the sign-in.
    #[serde(default)]
    nonce: Option<String>,
}

/// The part of the discovery document that the server reads. An issuer of CI jobs has no sign-in
/// endpoints.
#[derive(Deserialize)]
struct Discovery {
    /// The URL where the browser signs in.
    #[serde(default)]
    authorization_endpoint: String,
    /// The URL that exchanges a code for tokens.
    #[serde(default)]
    token_endpoint: String,
    /// The URL of the keys.
    jwks_uri: String,
}

/// The part of a token response that the server reads.
#[derive(Deserialize)]
struct TokenResponse {
    /// The ID token.
    id_token: String,
}

/// The discovery and the keys of one issuer.
struct Issuer {
    /// The discovery document.
    discovery: Discovery,
    /// The keys, fetched again when a token names an unknown key.
    keys: RwLock<JwkSet>,
}

/// The OIDC clients of the server: one discovery for each issuer, read at its first use.
pub struct OidcClients {
    /// The HTTP client.
    http: reqwest::Client,
    /// The issuers, by their URL.
    issuers: RwLock<HashMap<String, Arc<Issuer>>>,
}

impl OidcClients {
    /// Makes the clients, with no issuer yet.
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            issuers: RwLock::new(HashMap::new()),
        }
    }

    /// Returns the URL where the browser signs in, with the state, the nonce and the PKCE
    /// challenge.
    pub(super) async fn authorize_url(
        &self,
        settings: &OidcSettings,
        redirect_uri: &str,
        state: &str,
        nonce: &str,
        challenge: &str,
    ) -> Result<String, Error> {
        let issuer = self.issuer(&settings.issuer).await?;
        let scope = settings.scopes.join(" ");
        let params = [
            ("response_type", "code"),
            ("client_id", settings.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("scope", scope.as_str()),
            ("state", state),
            ("nonce", nonce),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
        ];
        let url = Url::parse_with_params(&issuer.discovery.authorization_endpoint, params)
            .map_err(|_| unavailable("the authorization endpoint is not a URL"))?;
        Ok(url.into())
    }

    /// Exchanges the code of a callback for an ID token, with the PKCE verifier.
    pub(super) async fn exchange(
        &self,
        settings: &OidcSettings,
        secret: Option<&str>,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> Result<String, Error> {
        let issuer = self.issuer(&settings.issuer).await?;
        let body = {
            let mut form = url::form_urlencoded::Serializer::new(String::new());
            form.append_pair("grant_type", "authorization_code")
                .append_pair("code", code)
                .append_pair("redirect_uri", redirect_uri)
                .append_pair("client_id", &settings.client_id)
                .append_pair("code_verifier", verifier);
            if let Some(secret) = secret {
                form.append_pair("client_secret", secret);
            }
            form.finish()
        };
        let request = self
            .http
            .post(&issuer.discovery.token_endpoint)
            .header("content-type", "application/x-www-form-urlencoded")
            .header("accept", "application/json")
            .body(body);
        let response: TokenResponse = send_exchange(request).await?;
        Ok(response.id_token)
    }

    /// Checks an ID token of a provider, and its nonce if the sign-in sent one. Only asymmetric
    /// signatures pass.
    pub(super) async fn verify(
        &self,
        settings: &OidcSettings,
        id_token: &str,
        nonce: Option<&str>,
    ) -> Result<ExternalIdentity, Error> {
        // Refuse a symmetric signature, then find the key of the token.
        let header = decode_header(id_token).map_err(|_| invalid())?;
        if matches!(
            header.alg,
            Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512
        ) {
            return Err(invalid());
        }
        let issuer = self.issuer(&settings.issuer).await?;
        let key = self.key(&issuer, &header.kid.ok_or_else(invalid)?).await?;

        // Verify the signature, the issuer, the audience, the expiry and the nonce.
        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[&settings.issuer]);
        validation.set_audience(&[&settings.client_id]);
        let claims = decode::<Claims>(id_token, &key, &validation)
            .map_err(|_| invalid())?
            .claims;
        if nonce.is_some() && claims.nonce.as_deref() != nonce {
            return Err(invalid());
        }
        Ok(ExternalIdentity {
            subject: claims.sub,
            email_verified: claims.email_verified.unwrap_or(false),
            email: claims.email,
            name: claims.name,
        })
    }

    /// Checks the ID token of a CI job of an issuer for an audience, and returns its claims. A
    /// token that does not pass gives `None`, and an issuer that does not answer an error. Only
    /// asymmetric signatures pass.
    pub(crate) async fn verify_ci(
        &self,
        issuer: &str,
        audience: &str,
        token: &str,
    ) -> Result<Option<HashMap<String, serde_json::Value>>, Error> {
        let Ok(header) = decode_header(token) else {
            return Ok(None);
        };
        let symmetric = [Algorithm::HS256, Algorithm::HS384, Algorithm::HS512];
        let Some(kid) = header.kid.filter(|_| !symmetric.contains(&header.alg)) else {
            return Ok(None);
        };
        let found = self.issuer(issuer).await?;
        let Ok(key) = self.key(&found, &kid).await else {
            return Ok(None);
        };
        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[issuer]);
        validation.set_audience(&[audience]);
        let claims = decode::<HashMap<String, serde_json::Value>>(token, &key, &validation);
        Ok(claims.ok().map(|data| data.claims))
    }

    /// Returns an issuer, and reads its discovery and its keys at its first use.
    async fn issuer(&self, url: &str) -> Result<Arc<Issuer>, Error> {
        if let Some(issuer) = self.issuers.read().await.get(url) {
            return Ok(Arc::clone(issuer));
        }
        let discovery_url = format!(
            "{}/.well-known/openid-configuration",
            url.trim_end_matches('/')
        );
        let discovery: Discovery = send(self.http.get(&discovery_url)).await?;
        let keys: JwkSet = send(self.http.get(&discovery.jwks_uri)).await?;
        let issuer = Arc::new(Issuer {
            discovery,
            keys: RwLock::new(keys),
        });
        self.issuers
            .write()
            .await
            .insert(url.to_owned(), Arc::clone(&issuer));
        Ok(issuer)
    }

    /// Returns the key of an id, and fetches the keys again one time if it is unknown.
    async fn key(&self, issuer: &Issuer, kid: &str) -> Result<DecodingKey, Error> {
        if let Some(jwk) = issuer.keys.read().await.find(kid) {
            return DecodingKey::from_jwk(jwk).map_err(|_| invalid());
        }
        let fresh: JwkSet = send(self.http.get(&issuer.discovery.jwks_uri)).await?;
        let key = fresh.find(kid).map(DecodingKey::from_jwk);
        *issuer.keys.write().await = fresh;
        key.ok_or_else(invalid)?.map_err(|_| invalid())
    }
}

/// The error of a token that does not pass. It gives no detail to the client.
fn invalid() -> Error {
    AccessError::Unauthenticated("invalid ID token".to_owned()).into()
}
