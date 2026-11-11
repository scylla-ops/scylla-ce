//! The calls of the connections to GitHub: the installations that a user sees, one installation
//! of the App, and the repositories of an installation.

use std::collections::BTreeMap;

use serde::Deserialize;
use time::OffsetDateTime;

use crate::infra::vcs::{Auth, GitHubApp, Hidden, ProviderError, TokenScope, VcsClients};

/// The most items of one page of GitHub.
const PAGE: u32 = 100;

/// An installation of the App, from `GET /app/installations/{id}`.
#[derive(Debug, Deserialize)]
pub(crate) struct Installation {
    /// The account of the installation.
    pub(crate) account: Account,
    /// `all` or `selected`.
    pub(crate) repository_selection: String,
    /// The permissions of the installation, by name: `checks` gives `write`.
    #[serde(default)]
    pub(crate) permissions: BTreeMap<String, String>,
    /// The time when an owner suspended the App, or `None`.
    pub(crate) suspended_at: Option<String>,
}

/// The account of an installation.
#[derive(Debug, Deserialize)]
pub(crate) struct Account {
    /// The login, for example `acme`.
    pub(crate) login: String,
    /// The address of the avatar.
    pub(crate) avatar_url: Option<String>,
    /// `Organization` or `User`.
    #[serde(rename = "type")]
    pub(crate) kind: String,
}

/// A repository of an installation.
#[derive(Debug, Deserialize)]
pub(crate) struct GitHubRepository {
    /// The id of the repository.
    pub(crate) id: u64,
    /// The full name, for example `acme/api`.
    pub(crate) full_name: String,
    /// The page of the repository.
    pub(crate) html_url: String,
    /// The default branch, or `None` for an empty repository.
    pub(crate) default_branch: Option<String>,
    /// `true` for a private repository.
    #[serde(default)]
    pub(crate) private: bool,
}

/// An installation in the list of a user.
#[derive(Deserialize)]
struct Listed {
    /// The id of the installation.
    id: u64,
}

/// The number of repositories of an installation.
#[derive(Deserialize)]
struct Count {
    /// The number of repositories.
    total_count: u32,
}

/// Returns `true` when the user of a token sees the installation: `GET /user/installations`.
pub(crate) async fn user_sees(
    clients: &VcsClients,
    user: &Hidden,
    installation: u64,
) -> Result<bool, ProviderError> {
    let listed: Vec<Listed> = clients
        .github
        .get("/user/installations")
        .auth(Auth::Bearer(user))
        .query("per_page", PAGE)
        .list(Some("installations"))
        .await?;
    Ok(listed.iter().any(|item| item.id == installation))
}

/// Reads an installation with the JWT of the App.
pub(crate) async fn installation(
    clients: &VcsClients,
    app: &GitHubApp,
    id: u64,
) -> Result<Installation, ProviderError> {
    let jwt = app.jwt(OffsetDateTime::now_utc())?;
    clients
        .github
        .get(&format!("/app/installations/{id}"))
        .auth(Auth::Bearer(&jwt))
        .read()
        .await
}

/// Returns the number of repositories of an installation.
pub(crate) async fn repository_count(
    clients: &VcsClients,
    app: &GitHubApp,
    installation: u64,
) -> Result<u32, ProviderError> {
    let token = app.installation_token(installation, &read_scope()).await?;
    let count: Count = clients
        .github
        .get("/installation/repositories")
        .auth(Auth::Bearer(&token.token))
        .query("per_page", 1)
        .read()
        .await?;
    Ok(count.total_count)
}

/// Returns each repository of an installation, in the order of GitHub.
pub(crate) async fn repositories(
    clients: &VcsClients,
    app: &GitHubApp,
    installation: u64,
) -> Result<Vec<GitHubRepository>, ProviderError> {
    let token = app.installation_token(installation, &read_scope()).await?;
    clients
        .github
        .get("/installation/repositories")
        .auth(Auth::Bearer(&token.token))
        .query("per_page", PAGE)
        .list(Some("repositories"))
        .await
}

/// The scope of a token that reads the repositories of an installation.
fn read_scope() -> TokenScope {
    TokenScope::default().permission("metadata", "read")
}
