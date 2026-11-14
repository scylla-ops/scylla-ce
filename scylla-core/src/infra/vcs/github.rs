//! The auth of a GitHub App: its JWT (RS256, signed with its private key), its installation
//! tokens narrowed to repositories and permissions, and the exchange of the code of a user.

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::{Duration, OffsetDateTime};

use super::{
    ApiClient, Auth, GitHubAppSettings, Hidden, InstallationToken, ProviderError, TokenCache,
    TokenScope,
};

/// The time before now in the `iat` of a JWT, against a drift of the clocks.
const CLOCK_SKEW: Duration = Duration::seconds(60);

/// The life of a JWT after now. GitHub refuses more than 10 minutes.
const JWT_LIFE: Duration = Duration::minutes(9);

/// The claims of the JWT of a GitHub App.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppClaims {
    /// The time of issue, in seconds since the epoch: one minute before now.
    pub iat: i64,
    /// The expiry, in seconds since the epoch: nine minutes after now.
    pub exp: i64,
    /// The issuer: the client id of the App.
    pub iss: String,
}

/// A GitHub App, with its key and the cache of its installation tokens.
pub struct GitHubApp {
    /// The ids and the secrets of the App.
    settings: GitHubAppSettings,
    /// The private key that signs the JWT.
    key: EncodingKey,
    /// The GitHub REST API.
    api: ApiClient,
    /// The web routes of GitHub, for the exchange of a code.
    web: ApiClient,
    /// The installation tokens.
    tokens: TokenCache,
}

/// The answer of `POST /app/installations/{id}/access_tokens`.
#[derive(Deserialize)]
struct TokenAnswer {
    /// The installation token.
    token: Hidden,
    /// The expiry, in RFC 3339.
    expires_at: String,
}

/// The answer of `POST /login/oauth/access_token`.
#[derive(Deserialize)]
struct CodeAnswer {
    /// The token of the user, on a success.
    access_token: Option<Hidden>,
    /// The code of a refusal, such as `bad_verification_code`.
    error: Option<String>,
}

impl GitHubApp {
    /// Makes an App from its settings, with the clients of the API and of the web routes.
    pub fn new(
        settings: &GitHubAppSettings,
        api: ApiClient,
        web: ApiClient,
    ) -> Result<Self, String> {
        Ok(Self {
            key: settings.key()?,
            settings: settings.clone(),
            api,
            web,
            tokens: TokenCache::default(),
        })
    }

    /// Returns the settings of the App.
    pub fn settings(&self) -> &GitHubAppSettings {
        &self.settings
    }

    /// Returns the claims of a JWT made at the time `now`.
    pub fn claims(&self, now: OffsetDateTime) -> AppClaims {
        AppClaims {
            iat: (now - CLOCK_SKEW).unix_timestamp(),
            exp: (now + JWT_LIFE).unix_timestamp(),
            iss: self.settings.client_id.clone(),
        }
    }

    /// Signs a JWT of the App at the time `now`, for the routes `/app/*`.
    pub fn jwt(&self, now: OffsetDateTime) -> Result<Hidden, ProviderError> {
        jsonwebtoken::encode(&Header::new(Algorithm::RS256), &self.claims(now), &self.key)
            .map(Hidden::new)
            .map_err(|_| ProviderError::Invalid("the JWT of the App cannot be signed".to_owned()))
    }

    /// Returns an installation token for a scope: from the cache while it serves, else a new one.
    pub async fn installation_token(
        &self,
        installation: u64,
        scope: &TokenScope,
    ) -> Result<InstallationToken, ProviderError> {
        // Give the token of the cache while it serves.
        let now = OffsetDateTime::now_utc();
        if let Some(token) = self.tokens.get(installation, scope, now) {
            return Ok(token);
        }

        // Ask GitHub for a token narrowed to the scope, and keep it.
        let jwt = self.jwt(now)?;
        let path = format!("/app/installations/{installation}/access_tokens");
        let answer: TokenAnswer = self
            .api
            .call(Method::POST, &path)
            .auth(Auth::Bearer(&jwt))
            .json(scope.body())
            .read()
            .await?;
        let token = InstallationToken {
            token: answer.token,
            expires_at: time_of(&answer.expires_at)?,
        };
        self.tokens
            .put(installation, scope.clone(), token.clone(), now);
        Ok(token)
    }

    /// Drops the tokens of an installation from the cache.
    pub fn forget(&self, installation: u64) {
        self.tokens.forget(installation);
    }

    /// Exchanges the code of a user after the install for the token of the user.
    pub async fn exchange_code(&self, code: &str) -> Result<Hidden, ProviderError> {
        // Send the code with the client id and the client secret of the App.
        let body = json!({
            "client_id": self.settings.client_id,
            "client_secret": self.settings.client_secret.expose(),
            "code": code,
        });
        let answer: CodeAnswer = self
            .web
            .call(Method::POST, "/login/oauth/access_token")
            .json(body)
            .read()
            .await?;

        // Take the token, or the code of the refusal.
        match (answer.access_token, answer.error) {
            (Some(token), None) => Ok(token),
            (_, error) => Err(ProviderError::Invalid(
                error.unwrap_or_else(|| "no token".to_owned()),
            )),
        }
    }
}

/// Reads a time in RFC 3339.
fn time_of(text: &str) -> Result<OffsetDateTime, ProviderError> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .and_then(|time| OffsetDateTime::from_unix_timestamp(time.timestamp()).ok())
        .ok_or_else(|| ProviderError::Invalid(format!("{text:?} is not a time")))
}
