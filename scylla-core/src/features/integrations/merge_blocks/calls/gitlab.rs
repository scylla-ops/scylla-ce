//! The calls of the merge blocks to GitLab with the token of a service account: the setting
//! "Pipelines must succeed" of a project.

use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::infra::vcs::{Auth, Hidden, ProviderError, VcsClients};

/// The merge settings of a project.
#[derive(Deserialize)]
struct Settings {
    /// `true` when a merge request needs a pipeline that passed.
    #[serde(default)]
    only_allow_merge_if_pipeline_succeeds: bool,
}

/// Turns "Pipelines must succeed" of a project on or off. GitLab needs the role Maintainer.
pub(crate) async fn set(
    clients: &VcsClients,
    token: &Hidden,
    project: &str,
    on: bool,
) -> Result<(), ProviderError> {
    let body = json!({ "only_allow_merge_if_pipeline_succeeds": on });
    let _: Value = clients
        .gitlab
        .call(Method::PUT, &format!("/projects/{project}"))
        .auth(Auth::PrivateToken(token))
        .json(body)
        .read()
        .await?;
    Ok(())
}

/// Returns `true` when "Pipelines must succeed" is on in a project.
pub(crate) async fn read(
    clients: &VcsClients,
    token: &Hidden,
    project: &str,
) -> Result<bool, ProviderError> {
    let settings: Settings = clients
        .gitlab
        .get(&format!("/projects/{project}"))
        .auth(Auth::PrivateToken(token))
        .read()
        .await?;
    Ok(settings.only_allow_merge_if_pipeline_succeeds)
}
