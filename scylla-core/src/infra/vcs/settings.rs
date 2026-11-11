//! The settings of the Git providers: the URLs of GitHub and GitLab, and the GitHub App of the
//! server, from command line flags or environment variables.

use std::fmt;
use std::path::PathBuf;

use clap::Args;
use jsonwebtoken::EncodingKey;

use super::Hidden;
use crate::Error;

/// The REST API of github.com.
pub const GITHUB_API_URL: &str = "https://api.github.com";

/// The web URL of github.com.
pub const GITHUB_WEB_URL: &str = "https://github.com";

/// The URL of gitlab.com.
pub const GITLAB_URL: &str = "https://gitlab.com";

/// The settings of the Git providers. Each flag also reads the environment variable in its doc.
/// The settings of the GitHub App are all set or all unset.
#[derive(Args, Clone)]
#[group(id = "vcs")]
pub struct VcsSettings {
    /// The URL of the GitHub REST API.
    #[arg(long, env = "SCYLLA_GITHUB_API_URL", default_value = GITHUB_API_URL)]
    pub github_api_url: String,

    /// The web URL of GitHub, for the install of the App and the exchange of a code.
    #[arg(long, env = "SCYLLA_GITHUB_WEB_URL", default_value = GITHUB_WEB_URL)]
    pub github_web_url: String,

    /// The URL of GitLab. Its REST API is at `/api/v4`.
    #[arg(long, env = "SCYLLA_GITLAB_URL", default_value = GITLAB_URL)]
    pub gitlab_url: String,

    /// The id of the GitHub App of the server.
    #[arg(long, env = "SCYLLA_GITHUB_APP_ID")]
    pub github_app_id: Option<u64>,

    /// The slug of the GitHub App, in its install URL.
    #[arg(long, env = "SCYLLA_GITHUB_APP_SLUG")]
    pub github_app_slug: Option<String>,

    /// The client id of the GitHub App: the issuer of its JWT.
    #[arg(long, env = "SCYLLA_GITHUB_APP_CLIENT_ID")]
    pub github_app_client_id: Option<String>,

    /// The client secret of the GitHub App, for the exchange of the code of a user.
    #[arg(long, env = "SCYLLA_GITHUB_APP_CLIENT_SECRET", hide_env_values = true)]
    pub github_app_client_secret: Option<String>,

    /// The PEM private key of the GitHub App. A `\n` in the value is a new line.
    #[arg(
        long,
        env = "SCYLLA_GITHUB_APP_PRIVATE_KEY",
        hide_env_values = true,
        conflicts_with = "github_app_private_key_file"
    )]
    pub github_app_private_key: Option<String>,

    /// A file with the PEM private key of the GitHub App, in place of the key in a value.
    #[arg(long, env = "SCYLLA_GITHUB_APP_PRIVATE_KEY_FILE")]
    pub github_app_private_key_file: Option<PathBuf>,

    /// The secret of the webhooks of the GitHub App.
    #[arg(long, env = "SCYLLA_GITHUB_APP_WEBHOOK_SECRET", hide_env_values = true)]
    pub github_app_webhook_secret: Option<String>,
}

/// The checked settings of the Git providers, for `ServeOptions`.
#[derive(Clone, Debug)]
pub struct VcsOptions {
    /// The URL of the GitHub REST API, with no `/` at the end.
    pub github_api_url: String,
    /// The web URL of GitHub, with no `/` at the end.
    pub github_web_url: String,
    /// The URL of GitLab, with no `/` at the end.
    pub gitlab_url: String,
    /// The GitHub App of the settings, or `None` when the server has none.
    pub github_app: Option<GitHubAppSettings>,
}

/// A GitHub App: its ids and its secrets.
#[derive(Clone, Debug)]
pub struct GitHubAppSettings {
    /// The id of the App.
    pub app_id: u64,
    /// The slug of the App, in its install URL.
    pub slug: String,
    /// The client id of the App: the issuer of its JWT.
    pub client_id: String,
    /// The client secret of the App.
    pub client_secret: Hidden,
    /// The PEM private key of the App.
    pub private_key: Hidden,
    /// The secret of the webhooks of the App.
    pub webhook_secret: Hidden,
}

impl Default for VcsOptions {
    fn default() -> Self {
        Self {
            github_api_url: GITHUB_API_URL.to_owned(),
            github_web_url: GITHUB_WEB_URL.to_owned(),
            gitlab_url: GITLAB_URL.to_owned(),
            github_app: None,
        }
    }
}

impl GitHubAppSettings {
    /// Reads the private key: an RSA key in PEM.
    pub fn key(&self) -> Result<EncodingKey, String> {
        EncodingKey::from_rsa_pem(self.private_key.expose().as_bytes())
            .map_err(|_| "must be an RSA private key in PEM".to_owned())
    }
}

impl VcsSettings {
    /// Checks the URLs and the GitHub App, and reads the private key of the App.
    pub fn resolve(&self) -> Result<VcsOptions, Error> {
        Ok(VcsOptions {
            github_api_url: base_url(&self.github_api_url, "SCYLLA_GITHUB_API_URL")?,
            github_web_url: base_url(&self.github_web_url, "SCYLLA_GITHUB_WEB_URL")?,
            gitlab_url: base_url(&self.gitlab_url, "SCYLLA_GITLAB_URL")?,
            github_app: self.github_app()?,
        })
    }

    /// Returns the GitHub App: all its settings, or none of them.
    fn github_app(&self) -> Result<Option<GitHubAppSettings>, Error> {
        // Refuse a part of the settings of the App.
        let key_source =
            self.github_app_private_key.is_some() || self.github_app_private_key_file.is_some();
        let parts = [
            ("SCYLLA_GITHUB_APP_ID", self.github_app_id.is_some()),
            ("SCYLLA_GITHUB_APP_SLUG", self.github_app_slug.is_some()),
            (
                "SCYLLA_GITHUB_APP_CLIENT_ID",
                self.github_app_client_id.is_some(),
            ),
            (
                "SCYLLA_GITHUB_APP_CLIENT_SECRET",
                self.github_app_client_secret.is_some(),
            ),
            ("SCYLLA_GITHUB_APP_PRIVATE_KEY", key_source),
            (
                "SCYLLA_GITHUB_APP_WEBHOOK_SECRET",
                self.github_app_webhook_secret.is_some(),
            ),
        ];
        if parts.iter().all(|(_, set)| !set) {
            return Ok(None);
        }
        if let Some(&(name, _)) = parts.iter().find(|(_, set)| !set) {
            return Err(Error::Config {
                name,
                reason: "is required when another SCYLLA_GITHUB_APP_* setting is set".to_owned(),
            });
        }

        // Read the key, and check that it is an RSA key.
        let app = GitHubAppSettings {
            app_id: self.github_app_id.unwrap_or_default(),
            slug: self.github_app_slug.clone().unwrap_or_default(),
            client_id: self.github_app_client_id.clone().unwrap_or_default(),
            client_secret: Hidden::new(self.github_app_client_secret.clone().unwrap_or_default()),
            private_key: Hidden::new(self.private_key()?),
            webhook_secret: Hidden::new(self.github_app_webhook_secret.clone().unwrap_or_default()),
        };
        app.key().map_err(|reason| Error::Config {
            name: "SCYLLA_GITHUB_APP_PRIVATE_KEY",
            reason,
        })?;
        Ok(Some(app))
    }

    /// Returns the PEM text of the private key, from the value or from the file.
    fn private_key(&self) -> Result<String, Error> {
        if let Some(text) = &self.github_app_private_key {
            return Ok(text.replace("\\n", "\n"));
        }
        let path = self.github_app_private_key_file.clone().unwrap_or_default();
        std::fs::read_to_string(path).map_err(|error| Error::Config {
            name: "SCYLLA_GITHUB_APP_PRIVATE_KEY_FILE",
            reason: format!("cannot be read: {error}"),
        })
    }
}

impl fmt::Debug for VcsSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hide = |value: &Option<String>| value.as_ref().map(|_| "***");
        f.debug_struct("VcsSettings")
            .field("github_api_url", &self.github_api_url)
            .field("github_web_url", &self.github_web_url)
            .field("gitlab_url", &self.gitlab_url)
            .field("github_app_id", &self.github_app_id)
            .field("github_app_slug", &self.github_app_slug)
            .field("github_app_client_id", &self.github_app_client_id)
            .field(
                "github_app_client_secret",
                &hide(&self.github_app_client_secret),
            )
            .field(
                "github_app_private_key",
                &hide(&self.github_app_private_key),
            )
            .field(
                "github_app_private_key_file",
                &self.github_app_private_key_file,
            )
            .field(
                "github_app_webhook_secret",
                &hide(&self.github_app_webhook_secret),
            )
            .finish()
    }
}

/// Checks an http or https URL, and removes the `/` at its end.
fn base_url(text: &str, name: &'static str) -> Result<String, Error> {
    match url::Url::parse(text) {
        Ok(url) if matches!(url.scheme(), "http" | "https") && url.has_host() => {
            Ok(text.trim_end_matches('/').to_owned())
        }
        _ => Err(Error::Config {
            name,
            reason: "must be an http or https URL".to_owned(),
        }),
    }
}
