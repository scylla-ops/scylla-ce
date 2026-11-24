//! The clients of the Git providers of the server, made one time from the settings.

use std::sync::Arc;

use super::{ApiClient, Flavor, GitHubApp, GitHubAppSettings, VcsOptions};
use crate::Error;

/// The clients of the Git providers.
#[derive(Clone)]
pub struct VcsClients {
    /// The GitHub REST API.
    pub github: ApiClient,
    /// The web routes of GitHub: the install of an App and the exchange of a code.
    pub github_web: ApiClient,
    /// The GitHub App of the settings, or `None` when the server has none.
    pub github_app: Option<Arc<GitHubApp>>,
    /// The GitLab REST API, at `/api/v4`. A call sends the token of a service account with
    /// `Auth::PrivateToken`.
    pub gitlab: ApiClient,
}

impl VcsClients {
    /// Makes the clients of the settings.
    pub fn new(options: &VcsOptions) -> Result<Self, Error> {
        // Make the clients of the APIs.
        let internal = |reason: String| Error::Internal(format!("a Git client: {reason}"));
        let github = ApiClient::new(Flavor::GitHub, &options.github_api_url).map_err(internal)?;
        let github_web = ApiClient::new(Flavor::Json, &options.github_web_url).map_err(internal)?;
        let gitlab_api = format!("{}/api/v4", options.gitlab_url);
        let gitlab = ApiClient::new(Flavor::Json, &gitlab_api).map_err(internal)?;
        let mut clients = Self {
            github,
            github_web,
            github_app: None,
            gitlab,
        };
        // Add the App of the settings, with its key.
        if let Some(settings) = &options.github_app {
            let app = clients.app(settings).map_err(|reason| Error::Config {
                name: "SCYLLA_GITHUB_APP_PRIVATE_KEY",
                reason,
            })?;
            clients.github_app = Some(Arc::new(app));
        }
        Ok(clients)
    }

    /// Makes a GitHub App with the clients of GitHub: the App of the settings, or the App of an
    /// instance.
    pub fn app(&self, settings: &GitHubAppSettings) -> Result<GitHubApp, String> {
        GitHubApp::new(settings, self.github.clone(), self.github_web.clone())
    }
}
