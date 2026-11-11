//! The logic of the connections that GitHub and GitLab share: the providers of a workspace, the
//! list, the read and the delete of a connection. One transaction for each call.

use scylla_domain::integrations::connections::{VcsConnection, VcsConnectionId};
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::{Page, PageRequest};

use super::calls::gitlab;
use super::credentials;
use super::store;
pub(super) use super::store::Counted;
use crate::features::integrations::apps::store as apps;
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// A Git provider, as a workspace can use it.
pub(super) struct ProviderInfo {
    /// The provider.
    pub(super) provider: VcsProvider,
    /// Why an admin cannot connect it now, as a stable code, or `None`.
    pub(super) unavailable: Option<&'static str>,
    /// The address of the provider.
    pub(super) web_url: String,
}

/// Returns GitHub, then GitLab, with what a workspace needs to connect each one: a GitHub App,
/// and a key of the secrets to seal a GitLab token.
pub(super) async fn providers(ctx: &IntegrationContext) -> Result<Vec<ProviderInfo>, Error> {
    // Find the GitHub App: the App of the settings, else the App of the instance.
    let github = match &ctx.clients.github_app {
        Some(_) => None,
        None => {
            let mut tx = ctx.pool.begin().await?;
            let row = apps::github(&mut tx).await?;
            tx.commit().await?;
            match (row, &ctx.sealer) {
                (Some(_), Some(_)) => None,
                (Some(_), None) => Some("SECRET_KEY_MISSING"),
                (None, _) => Some("APP_MISSING"),
            }
        }
    };

    // GitLab needs the key of the secrets for its token.
    let gitlab = ctx.sealer.is_none().then_some("SECRET_KEY_MISSING");
    Ok(vec![
        ProviderInfo {
            provider: VcsProvider::GitHub,
            unavailable: github,
            web_url: ctx.settings.github_web_url.clone(),
        },
        ProviderInfo {
            provider: VcsProvider::GitLab,
            unavailable: gitlab,
            web_url: ctx.settings.gitlab_url.clone(),
        },
    ])
}

/// Returns a connection and the number of its projects, or `None` if no connection has this id.
pub(super) async fn get(
    ctx: &IntegrationContext,
    id: VcsConnectionId,
) -> Result<Option<Counted>, Error> {
    let mut tx = ctx.pool.begin().await?;
    let found = store::find(&mut tx, id).await?;
    tx.commit().await?;
    Ok(found)
}

/// Returns one page of the connections of a workspace, oldest first.
pub(super) async fn list(
    ctx: &IntegrationContext,
    workspace: WorkspaceId,
    page: PageRequest<VcsConnectionId>,
) -> Result<Page<Counted, VcsConnectionId>, Error> {
    let mut tx = ctx.pool.begin().await?;
    let rows = store::list(&mut tx, workspace, &page).await?;
    tx.commit().await?;
    Ok(Page::from_overfetch(rows, page.size, |found| {
        found.connection.id
    }))
}

/// Returns the workspace of a project, or `None` if no project has this id.
pub(super) async fn project_workspace(
    ctx: &IntegrationContext,
    project: ProjectId,
) -> Result<Option<WorkspaceId>, Error> {
    let mut tx = ctx.pool.begin().await?;
    let workspace = store::project_workspace(&mut tx, project).await?;
    tx.commit().await?;
    Ok(workspace)
}

/// Deletes a connection with the links of its repositories. The webhooks of GitLab go first,
/// when the server can delete them. The jobs stay.
pub(super) async fn delete(ctx: &IntegrationContext, found: &Counted) -> Result<(), Error> {
    // Delete the webhooks that Scylla made on GitLab.
    let connection = &found.connection;
    if connection.provider == VcsProvider::GitLab {
        delete_hooks(ctx, connection).await?;
    }

    // Delete the connection, then drop the tokens of its installation.
    let mut tx = ctx.pool.begin().await?;
    if !store::delete(&mut tx, connection.id).await? {
        return Err(Error::NotFound("connection"));
    }
    tx.commit().await?;
    if let (Some(app), None) = (&ctx.clients.github_app, connection.app_id) {
        if let Ok(installation) = connection.external_id.parse() {
            app.forget(installation);
        }
    }
    tracing::info!(
        event = "vcs.disconnected",
        connection = %connection.id,
        workspace = %connection.workspace_id,
        provider = connection.provider.as_str(),
        account = %connection.account,
        projects = found.projects,
    );
    Ok(())
}

/// Deletes the webhooks of the links of a GitLab connection. A failure goes to the log only.
async fn delete_hooks(ctx: &IntegrationContext, connection: &VcsConnection) -> Result<(), Error> {
    // Read the hooks, and open the token.
    let mut tx = ctx.pool.begin().await?;
    let hooks = store::hooks(&mut tx, connection.id).await?;
    tx.commit().await?;
    if hooks.is_empty() {
        return Ok(());
    }
    let token = match credentials::gitlab_token(ctx, connection.id).await {
        Ok(token) => token,
        Err(error) => {
            tracing::warn!(connection = %connection.id, %error, "the GitLab webhooks stay");
            return Ok(());
        }
    };

    // Delete each hook.
    for (project, hook) in hooks {
        if let Err(error) = gitlab::delete_hook(&ctx.clients, &token, &project, &hook).await {
            tracing::warn!(
                connection = %connection.id,
                project = %project,
                %error,
                "a GitLab webhook stays"
            );
        }
    }
    Ok(())
}
