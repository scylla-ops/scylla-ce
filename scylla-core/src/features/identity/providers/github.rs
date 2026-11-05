//! The GitHub client: OAuth 2 with PKCE, then the API for the user and its verified email.
//! GitHub gives no ID token, so the server reads the user with the access token.

use serde::{Deserialize, Serialize};
use url::Url;

use super::calls::{send, send_exchange};
use super::ExternalIdentity;
use crate::Error;

/// The settings of a GitHub provider, in its JSON. GitHub Enterprise Server has other URLs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitHubSettings {
    /// The client id of the OAuth app.
    pub client_id: String,
    /// The web URL, for the sign-in and the code exchange.
    #[serde(default = "default_web_url")]
    pub web_url: String,
    /// The URL of the REST API.
    #[serde(default = "default_api_url")]
    pub api_url: String,
}

/// The web URL of github.com.
fn default_web_url() -> String {
    "https://github.com".to_owned()
}

/// The API URL of github.com.
fn default_api_url() -> String {
    "https://api.github.com".to_owned()
}

/// The answer of the code exchange.
#[derive(Deserialize)]
struct TokenResponse {
    /// The access token of the user.
    access_token: String,
}

/// The user of the API.
#[derive(Deserialize)]
struct GitHubUser {
    /// The id of the user, which never changes.
    id: u64,
    /// The login, which can change.
    login: String,
    /// The name, if the user set one.
    #[serde(default)]
    name: Option<String>,
}

/// An email of the user in the API.
#[derive(Deserialize)]
struct GitHubEmail {
    /// The address.
    email: String,
    /// `true` for the primary email.
    primary: bool,
    /// `true` if GitHub verified it.
    verified: bool,
}

/// Returns the URL where the browser signs in, with the state and the PKCE challenge.
pub(super) fn authorize_url(
    settings: &GitHubSettings,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
) -> Result<String, Error> {
    let params = [
        ("client_id", settings.client_id.as_str()),
        ("redirect_uri", redirect_uri),
        ("scope", "read:user user:email"),
        ("state", state),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
    ];
    let base = format!(
        "{}/login/oauth/authorize",
        settings.web_url.trim_end_matches('/')
    );
    let url = Url::parse_with_params(&base, params)
        .map_err(|_| Error::Internal("the GitHub URL is not valid".to_owned()))?;
    Ok(url.into())
}

/// Exchanges the code of a callback, then reads the user and its primary verified email.
pub(super) async fn identity(
    http: &reqwest::Client,
    settings: &GitHubSettings,
    secret: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<ExternalIdentity, Error> {
    // Exchange the code for an access token.
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", &settings.client_id)
        .append_pair("client_secret", secret)
        .append_pair("code", code)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("code_verifier", verifier)
        .finish();
    let web = settings.web_url.trim_end_matches('/');
    let request = http
        .post(format!("{web}/login/oauth/access_token"))
        .header("content-type", "application/x-www-form-urlencoded")
        .header("accept", "application/json")
        .body(body);
    let token: TokenResponse = send_exchange(request).await?;

    // Read the user and its emails with the access token.
    let api = settings.api_url.trim_end_matches('/');
    let get = |path: &str| {
        http.get(format!("{api}{path}"))
            .bearer_auth(&token.access_token)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "scylla")
    };
    let user: GitHubUser = send(get("/user")).await?;
    let emails: Vec<GitHubEmail> = send(get("/user/emails")).await?;
    let email = emails
        .into_iter()
        .find(|email| email.primary && email.verified)
        .map(|email| email.email);
    Ok(ExternalIdentity {
        subject: user.id.to_string(),
        email_verified: email.is_some(),
        email,
        name: Some(user.name.unwrap_or(user.login)),
    })
}
