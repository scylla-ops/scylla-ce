//! The clients of the Git providers: their settings, one generic REST client with the pages, the
//! ETags and the limits, the auth of the GitHub App and of a GitLab token, and the checks of the
//! signatures of their webhooks. No token goes to `Debug` or to a log line.

mod call;
mod client;
mod clients;
mod error;
mod github;
mod hidden;
mod settings;
mod signatures;
#[cfg(test)]
mod tests;
mod tokens;

pub use call::{next_link, Answer, Call};
pub use client::{ApiClient, Auth, Flavor};
pub use clients::VcsClients;
pub use error::ProviderError;
pub use github::{AppClaims, GitHubApp};
pub use hidden::Hidden;
pub use settings::{
    GitHubAppSettings, VcsOptions, VcsSettings, GITHUB_API_URL, GITHUB_WEB_URL, GITLAB_URL,
};
pub use signatures::{
    check_github, check_gitlab, gitlab_signing_token, sign_github, sign_gitlab, GitLabDelivery,
    SignatureError, GITLAB_WINDOW,
};
pub use tokens::{InstallationToken, TokenCache, TokenScope, REFRESH_MARGIN};
