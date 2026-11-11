//! The calls of the repository links to GitHub: the branches and the tags of a repository, with
//! an installation token narrowed to the repository and to `contents: read`.

use serde::Deserialize;

use crate::infra::vcs::{Auth, GitHubApp, ProviderError, TokenScope, VcsClients};

/// The most items of one page of GitHub.
const PAGE: u32 = 100;

/// A branch or a tag of a repository.
#[derive(Debug, Deserialize)]
pub(crate) struct GitHubRef {
    /// The name, for example `main` or `v1.2.0`.
    pub(crate) name: String,
    /// The commit of the ref.
    pub(crate) commit: Commit,
}

/// The commit of a ref.
#[derive(Debug, Deserialize)]
pub(crate) struct Commit {
    /// The full SHA.
    pub(crate) sha: String,
}

/// The repository of a call: the installation that sees it, its id and its full name.
pub(crate) struct Repository<'a> {
    /// The id of the installation.
    pub(crate) installation: u64,
    /// The id of the repository.
    pub(crate) id: u64,
    /// The full name, for example `acme/api`.
    pub(crate) full_name: &'a str,
}

/// Returns each branch (`branches`) or each tag (`tags`) of a repository, in the order of
/// GitHub.
pub(crate) async fn refs(
    clients: &VcsClients,
    app: &GitHubApp,
    repository: &Repository<'_>,
    kind: &str,
) -> Result<Vec<GitHubRef>, ProviderError> {
    let scope = TokenScope::default()
        .repository(repository.id)
        .permission("contents", "read");
    let token = app
        .installation_token(repository.installation, &scope)
        .await?;
    let path = format!("/repos/{}/{kind}", repository.full_name);
    let listed = clients
        .github
        .get(&path)
        .auth(Auth::Bearer(&token.token))
        .query("per_page", PAGE)
        .list(None)
        .await;
    if matches!(listed, Err(ProviderError::Unauthorized)) {
        app.forget(repository.installation);
    }
    listed
}
