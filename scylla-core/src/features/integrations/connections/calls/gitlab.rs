//! The calls of the connections to GitLab with the token of a service account: the token itself,
//! its user, the projects that it sees, and the delete of a webhook.

use reqwest::Method;
use serde::Deserialize;
use serde_json::Value;

use crate::infra::vcs::{Auth, Hidden, ProviderError, VcsClients};

/// The most items of one page of GitLab.
const PAGE: u32 = 100;

/// The access level of a Developer: the least role that a connection lists.
const DEVELOPER: u32 = 30;

/// A token, from `GET /personal_access_tokens/self`.
#[derive(Debug, Deserialize)]
pub(crate) struct TokenInfo {
    /// The scopes, for example `api`.
    #[serde(default)]
    pub(crate) scopes: Vec<String>,
    /// The expiry date, for example `2027-11-01`, or `None` for no expiry.
    pub(crate) expires_at: Option<String>,
    /// `false` for a token that expired or was revoked.
    #[serde(default = "active")]
    pub(crate) active: bool,
    /// `true` for a revoked token.
    #[serde(default)]
    pub(crate) revoked: bool,
}

/// The user of a token, from `GET /user`.
#[derive(Debug, Deserialize)]
pub(crate) struct GitLabUser {
    /// The id of the user.
    pub(crate) id: u64,
    /// The login, for example `scylla-bot`.
    pub(crate) username: String,
    /// The address of the avatar.
    pub(crate) avatar_url: Option<String>,
    /// `true` for a service account, or the bot user of a group or project access token.
    #[serde(default)]
    pub(crate) bot: bool,
}

/// A project that the token sees.
#[derive(Debug, Deserialize)]
pub(crate) struct GitLabProject {
    /// The id of the project.
    pub(crate) id: u64,
    /// The full path, for example `bank-platform/payments`.
    pub(crate) path_with_namespace: String,
    /// The page of the project.
    pub(crate) web_url: String,
    /// The default branch, or `None` for an empty project.
    pub(crate) default_branch: Option<String>,
    /// `private`, `internal` or `public`.
    pub(crate) visibility: Option<String>,
    /// The access of the user of the token.
    pub(crate) permissions: Option<Permissions>,
}

/// The access of the user of a token to a project: as a member of the project, and as a member
/// of its group.
#[derive(Debug, Deserialize)]
pub(crate) struct Permissions {
    /// The access as a member of the project.
    project_access: Option<Access>,
    /// The access as a member of the group.
    group_access: Option<Access>,
}

/// One access of a user to a project.
#[derive(Debug, Deserialize)]
struct Access {
    /// 30 is Developer, 40 Maintainer, 50 Owner.
    access_level: u32,
}

impl GitLabProject {
    /// Returns the access level of the user of the token: the larger of its two accesses.
    pub(crate) fn level(&self) -> u32 {
        let level = |access: &Option<Access>| access.as_ref().map_or(0, |a| a.access_level);
        self.permissions.as_ref().map_or(0, |permissions| {
            level(&permissions.project_access).max(level(&permissions.group_access))
        })
    }

    /// Returns `true` for a private or internal project.
    pub(crate) fn private(&self) -> bool {
        self.visibility.as_deref() != Some("public")
    }
}

/// The default of `active`: a token is active when GitLab does not tell.
fn active() -> bool {
    true
}

/// Reads a token: its scopes and its expiry.
pub(crate) async fn token(
    clients: &VcsClients,
    token: &Hidden,
) -> Result<TokenInfo, ProviderError> {
    clients
        .gitlab
        .get("/personal_access_tokens/self")
        .auth(Auth::PrivateToken(token))
        .read()
        .await
}

/// Reads the user of a token.
pub(crate) async fn user(
    clients: &VcsClients,
    token: &Hidden,
) -> Result<GitLabUser, ProviderError> {
    clients
        .gitlab
        .get("/user")
        .auth(Auth::PrivateToken(token))
        .read()
        .await
}

/// Returns each project where the user of the token is Developer or more, in the order of the
/// ids. A search keeps the projects with the text in their path or in the path of their group.
pub(crate) async fn projects(
    clients: &VcsClients,
    token: &Hidden,
    search: &str,
) -> Result<Vec<GitLabProject>, ProviderError> {
    let mut call = clients
        .gitlab
        .get("/projects")
        .auth(Auth::PrivateToken(token))
        .query("membership", true)
        .query("min_access_level", DEVELOPER)
        .query("order_by", "id")
        .query("sort", "asc")
        .query("per_page", PAGE);
    if !search.is_empty() {
        call = call
            .query("search", search)
            .query("search_namespaces", true);
    }
    call.list(None).await
}

/// Deletes a webhook of a project.
pub(crate) async fn delete_hook(
    clients: &VcsClients,
    token: &Hidden,
    project: &str,
    hook: &str,
) -> Result<(), ProviderError> {
    clients
        .gitlab
        .call(Method::DELETE, &format!("/projects/{project}/hooks/{hook}"))
        .auth(Auth::PrivateToken(token))
        .send::<Value>()
        .await?;
    Ok(())
}
