//! The calls of the repository links to GitLab with the token of a service account: one project,
//! the webhook of a project, and the branches and the tags of a project.

use reqwest::Method;
use serde::Deserialize;
use serde_json::json;

use crate::features::integrations::connections::calls::gitlab::GitLabProject;
use crate::infra::vcs::{Auth, Hidden, ProviderError, VcsClients};

/// The most items of one page of GitLab.
const PAGE: u32 = 100;

/// The name of the webhooks that Scylla makes.
const HOOK_NAME: &str = "Scylla";

/// A branch or a tag of a project.
#[derive(Debug, Deserialize)]
pub(crate) struct GitLabRef {
    /// The name, for example `main` or `v1.2.0`.
    pub(crate) name: String,
    /// The commit of the ref.
    pub(crate) commit: Commit,
    /// The page of a branch. GitLab gives no page for a tag.
    pub(crate) web_url: Option<String>,
}

/// The commit of a ref.
#[derive(Debug, Deserialize)]
pub(crate) struct Commit {
    /// The full SHA.
    pub(crate) id: String,
}

/// A webhook that GitLab made.
#[derive(Deserialize)]
struct Hook {
    /// The id of the webhook.
    id: u64,
}

/// Reads a project, with the access of the user of the token.
pub(crate) async fn project(
    clients: &VcsClients,
    token: &Hidden,
    project: &str,
) -> Result<GitLabProject, ProviderError> {
    clients
        .gitlab
        .get(&format!("/projects/{project}"))
        .auth(Auth::PrivateToken(token))
        .read()
        .await
}

/// Makes the webhook of a project for the pushes, the tags and the merge requests, signed with
/// a signing token. Returns the id of the webhook.
pub(crate) async fn create_hook(
    clients: &VcsClients,
    token: &Hidden,
    project: &str,
    url: &str,
    signing_token: &Hidden,
) -> Result<String, ProviderError> {
    let body = json!({
        "url": url,
        "name": HOOK_NAME,
        "description": "The runs and the reports of Scylla",
        "push_events": true,
        "tag_push_events": true,
        "merge_requests_events": true,
        "enable_ssl_verification": true,
        "signing_token": signing_token.expose(),
    });
    let hook: Hook = clients
        .gitlab
        .call(Method::POST, &format!("/projects/{project}/hooks"))
        .auth(Auth::PrivateToken(token))
        .json(body)
        .read()
        .await?;
    Ok(hook.id.to_string())
}

/// Returns each branch (`branches`) or each tag (`tags`) of a project with this text in its
/// name, in the order of GitLab.
pub(crate) async fn refs(
    clients: &VcsClients,
    token: &Hidden,
    project: &str,
    kind: &str,
    search: &str,
) -> Result<Vec<GitLabRef>, ProviderError> {
    let mut call = clients
        .gitlab
        .get(&format!("/projects/{project}/repository/{kind}"))
        .auth(Auth::PrivateToken(token))
        .query("per_page", PAGE);
    if !search.is_empty() {
        call = call.query("search", search);
    }
    call.list(None).await
}
