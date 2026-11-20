//! The installations of the GitHub App that a user can connect to a workspace, after the
//! authorize page of a new install: the installations that the user sees on GitHub and that have
//! no connection in the workspace, the page that connects each one, and the install page.

use scylla_domain::integrations::connections::AccountKind;
use scylla_domain::tenancy::workspaces::WorkspaceId;

use super::calls::github;
use super::credentials;
use super::install;
use super::redirects;
use super::store::installations;
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// The installations that a user can connect, and the install page for another account.
pub(super) struct Choice {
    /// The installations, in the order of GitHub.
    pub(super) options: Vec<Choosable>,
    /// The install page of the App, with the state.
    pub(super) install_url: String,
}

/// An installation that a user can connect.
pub(super) struct Choosable {
    /// The id of the installation.
    pub(super) installation: u64,
    /// The login of the account.
    pub(super) account: String,
    /// The kind of the account.
    pub(super) account_kind: AccountKind,
    /// The address of the avatar of the account.
    pub(super) avatar_url: Option<String>,
    /// The authorize page of the App, with the state that carries the installation.
    pub(super) connect_url: String,
}

/// Returns the installations that the user of a code can connect to a workspace. The state stays
/// open: the page of an installation or the install page uses it. A code that GitHub refuses
/// gives `STATE_EXPIRED`, and an instance with no App gives `APP_MISSING`.
pub(super) async fn list(
    ctx: &IntegrationContext,
    workspace: WorkspaceId,
    state: &str,
    code: &str,
) -> Result<Choice, Error> {
    // Read the installations that the user sees on GitHub.
    let source = credentials::github_app(ctx).await?;
    let source = source.ok_or_else(credentials::app_missing)?;
    let user = install::user_token(&source, code).await?;
    let seen = github::user_installations(&ctx.clients, &user)
        .await
        .map_err(install::failed)?;

    // Find the installations that have a connection in the workspace.
    let mut tx = ctx.pool.begin().await?;
    let connected = installations::of_workspace(&mut tx, source.row, workspace).await?;
    tx.commit().await?;

    // Make the page of each other installation, and the install page.
    let mut options = Vec::new();
    for found in seen {
        if connected.contains(&found.id.to_string()) {
            continue;
        }
        let carried = redirects::carry(state, found.id);
        options.push(Choosable {
            installation: found.id,
            account_kind: found.account.account_kind(),
            account: found.account.login,
            avatar_url: found.account.avatar_url,
            connect_url: redirects::authorize_page(ctx, &source, &carried)?,
        });
    }
    Ok(Choice {
        options,
        install_url: redirects::install_page(ctx, &source, state)?,
    })
}
