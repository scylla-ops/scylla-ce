//! The check of an OIDC ID token: issuer, audience, expiry, and a signature from the JWKS.

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use scylla_extension::AccessError;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use tokio::sync::RwLock;

/// The identity in a valid ID token.
pub(crate) struct Identity {
    /// The issuer.
    pub(crate) issuer: String,
    /// The subject: the id of the person at the issuer.
    pub(crate) subject: String,
}

/// The claims that the server reads.
#[derive(Deserialize)]
struct Claims {
    /// The subject.
    sub: String,
}

/// The part of the discovery document that the server reads.
#[derive(Deserialize)]
struct Discovery {
    /// The URL of the keys of the issuer.
    jwks_uri: String,
}

/// Checks the ID tokens of one issuer and one audience.
pub struct OidcVerifier {
    /// The issuer, as in the `iss` claim.
    issuer: String,
    /// The audience: the client id of Scylla at the issuer.
    audience: String,
    /// The URL of the keys.
    jwks_uri: String,
    /// The HTTP client.
    http: reqwest::Client,
    /// The keys, fetched again when a token names an unknown key.
    keys: RwLock<JwkSet>,
}

impl OidcVerifier {
    /// Reads the discovery document and the keys of the issuer.
    pub async fn discover(issuer: &str, audience: &str) -> Result<Self, String> {
        // Read the discovery document of the issuer.
        crate::crypto::install();
        let http = reqwest::Client::new();
        let url = format!(
            "{}/.well-known/openid-configuration",
            issuer.trim_end_matches('/')
        );
        let discovery: Discovery = fetch(&http, &url).await?;

        // Read the keys that it names, and keep their address for a key rotation.
        let keys: JwkSet = fetch(&http, &discovery.jwks_uri).await?;
        Ok(Self {
            issuer: issuer.to_owned(),
            audience: audience.to_owned(),
            jwks_uri: discovery.jwks_uri,
            http,
            keys: RwLock::new(keys),
        })
    }

    /// Checks an ID token and returns its identity. Only asymmetric signatures pass.
    pub(crate) async fn verify(&self, id_token: &str) -> Result<Identity, AccessError> {
        // Refuse a symmetric signature.
        let header = decode_header(id_token).map_err(|_| invalid())?;
        if matches!(
            header.alg,
            Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512
        ) {
            return Err(invalid());
        }

        // Verify the signature with the key of the issuer, then the issuer and the audience.
        let kid = header.kid.ok_or_else(invalid)?;
        let key = self.key(&kid).await?;
        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[&self.issuer]);
        validation.set_audience(&[&self.audience]);
        let claims = decode::<Claims>(id_token, &key, &validation)
            .map_err(|_| invalid())?
            .claims;
        Ok(Identity {
            issuer: self.issuer.clone(),
            subject: claims.sub,
        })
    }

    /// Returns the key of an id, and fetches the keys again one time if it is unknown.
    async fn key(&self, kid: &str) -> Result<DecodingKey, AccessError> {
        if let Some(jwk) = self.keys.read().await.find(kid) {
            return DecodingKey::from_jwk(jwk).map_err(|_| invalid());
        }
        let fresh: JwkSet = fetch(&self.http, &self.jwks_uri)
            .await
            .map_err(AccessError::Unavailable)?;
        let key = fresh.find(kid).map(DecodingKey::from_jwk);
        *self.keys.write().await = fresh;
        key.ok_or_else(invalid)?.map_err(|_| invalid())
    }
}

/// Reads a JSON document over HTTP.
async fn fetch<T: DeserializeOwned>(http: &reqwest::Client, url: &str) -> Result<T, String> {
    let response = http
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| format!("cannot read {url}: {error}"))?;
    response
        .json()
        .await
        .map_err(|error| format!("cannot read {url}: {error}"))
}

/// The error of a token that does not pass. It gives no detail to the client.
fn invalid() -> AccessError {
    AccessError::Unauthenticated("invalid ID token".to_owned())
}
