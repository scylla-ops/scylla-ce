//! The redirects of GitHub that are not a new install: an install on an account where the App is
//! installed comes back with no code, so the user authorizes the App on the authorize page of
//! GitHub first; a change of an installation on GitHub comes back with no state, and refreshes
//! the connections of the installation.

use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Rejection;
use url::Url;

use super::credentials;
use super::install;
use super::store::installations;
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// The mark between a state and the installation that it carries to the authorize page.
const MARK: char = '.';

/// Splits the state of a redirect into the state of `StartGitHubInstall` and the installation
/// that it carries back from the authorize page, if any. A bad installation gives
/// `STATE_EXPIRED`.
pub(super) fn split(state: &str) -> Result<(&str, Option<u64>), Error> {
    match state.split_once(MARK) {
        None => Ok((state, None)),
        Some((state, installation)) => installation
            .parse()
            .map(|installation| (state, Some(installation)))
            .map_err(|_| install::state_expired()),
    }
}

/// Returns the refusal `AUTHORIZATION_REQUIRED` with the authorize page of GitHub in
/// `authorize_url`: the state carries the installation there, and GitHub comes back to the
/// callback with a code. A failure to make the page gives its own error.
pub(super) async fn authorize(ctx: &IntegrationContext, state: &str, installation: u64) -> Error {
    match page(ctx, state, installation).await {
        Ok(url) => {
            let message = "the user must authorize the GitHub App: open authorize_url";
            let rejection = Rejection::new("AUTHORIZATION_REQUIRED", message);
            Error::Rejected(rejection.detail("authorize_url", url))
        }
        Err(error) => error,
    }
}

/// Returns the workspaces with a connection of an installation of the GitHub App, the oldest
/// connection first. An instance with no App gives `APP_MISSING`.
pub(super) async fn workspaces(
    ctx: &IntegrationContext,
    installation: u64,
) -> Result<Vec<WorkspaceId>, Error> {
    let source = credentials::github_app(ctx).await?;
    let source = source.ok_or_else(credentials::app_missing)?;
    let mut tx = ctx.pool.begin().await?;
    let found = installations::workspaces(&mut tx, source.row, &installation.to_string()).await?;
    tx.commit().await?;
    Ok(found)
}

/// Makes the authorize page of the App, with its callback on this server and the state that
/// carries the installation.
async fn page(ctx: &IntegrationContext, state: &str, installation: u64) -> Result<String, Error> {
    // Find the App.
    let source = credentials::github_app(ctx).await?;
    let source = source.ok_or_else(credentials::app_missing)?;

    // Make the page.
    let page = format!("{}/login/oauth/authorize", ctx.settings.github_web_url);
    let mut url = Url::parse(&page).map_err(|_| Error::Internal(format!("{page} is not a URL")))?;
    let app_url = ctx.urls.app_url.trim_end_matches('/');
    url.query_pairs_mut()
        .append_pair("client_id", &source.app.settings().client_id)
        .append_pair(
            "redirect_uri",
            &format!("{app_url}/integrations/github/callback"),
        )
        .append_pair("state", &format!("{state}{MARK}{installation}"));
    Ok(url.into())
}
