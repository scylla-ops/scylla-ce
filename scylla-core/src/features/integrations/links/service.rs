//! The logic of the repository links: the read, the change and the delete of a link, and the
//! connection of a link. One transaction for each call.

use scylla_domain::integrations::connections::{VcsConnection, VcsConnectionId};
use scylla_domain::integrations::links::ReportMode;
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::ProjectId;
use scylla_extension::Rejection;

pub(super) use super::store::Changes;
use super::store::{self, Linked};
use crate::features::integrations::connections::calls::gitlab;
use crate::features::integrations::connections::credentials;
use crate::features::integrations::connections::store as connections;
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// Returns the link of a project, or `None` for a project with no link.
pub(super) async fn get(
    ctx: &IntegrationContext,
    project: ProjectId,
) -> Result<Option<Linked>, Error> {
    let mut tx = ctx.pool.begin().await?;
    let linked = store::find(&mut tx, project).await?;
    tx.commit().await?;
    Ok(linked)
}

/// Returns the link of a project. A project with no link gives `NO_REPOSITORY_LINK`.
pub(super) async fn require(ctx: &IntegrationContext, project: ProjectId) -> Result<Linked, Error> {
    get(ctx, project).await?.ok_or_else(|| {
        let message = "the project has no linked repository";
        Error::Rejected(Rejection::new("NO_REPOSITORY_LINK", message))
    })
}

/// Returns the connection of a link.
pub(super) async fn connection(
    ctx: &IntegrationContext,
    linked: &Linked,
) -> Result<VcsConnection, Error> {
    let mut tx = ctx.pool.begin().await?;
    let found = connections::find(&mut tx, linked.link.connection_id).await?;
    tx.commit().await?;
    let found = found.ok_or(Error::NotFound("connection"))?;
    Ok(found.connection)
}

/// Changes the reports and the polling of the link of a project, and returns the link. A check
/// run on GitLab is a bad request.
pub(super) async fn update(
    ctx: &IntegrationContext,
    project: ProjectId,
    changes: Changes,
) -> Result<Linked, Error> {
    // Refuse a project with no link, and a check run on GitLab.
    let linked = require(ctx, project).await?;
    if linked.provider == VcsProvider::GitLab && changes.report_mode == Some(ReportMode::CheckRun) {
        return Err(Error::invalid(
            "report_mode",
            "must be a commit status on GitLab",
        ));
    }

    // Change the link, and read it back.
    let mut tx = ctx.pool.begin().await?;
    let updated = store::update(&mut tx, project, changes).await?;
    let linked = store::find(&mut tx, project).await?;
    tx.commit().await?;
    match linked.filter(|_| updated) {
        Some(linked) => Ok(linked),
        None => require(ctx, project).await,
    }
}

/// Removes the link of a project, then the GitLab webhook that Scylla made. The Git triggers and
/// the merge rules of the project stop. The jobs stay.
pub(super) async fn unlink(ctx: &IntegrationContext, project: ProjectId) -> Result<(), Error> {
    // Delete the link.
    let linked = require(ctx, project).await?;
    let mut tx = ctx.pool.begin().await?;
    let unlinked = store::delete(&mut tx, project).await?;
    tx.commit().await?;
    let Some(unlinked) = unlinked else {
        return require(ctx, project).await.map(|_| ());
    };

    // Delete the webhook, when the server can.
    if let Some(hook) = &unlinked.hook_id {
        delete_hook(ctx, unlinked.connection, &unlinked.external_id, hook).await;
    }
    tracing::info!(
        event = "repository.unlinked",
        %project,
        workspace = %linked.link.workspace_id,
        connection = %unlinked.connection,
        provider = linked.provider.as_str(),
        repository = %unlinked.full_name,
        git_triggers = unlinked.git_triggers,
        merge_blocks = unlinked.merge_blocks,
    );
    Ok(())
}

/// Deletes a webhook of a GitLab project. A failure goes to the log only.
pub(super) async fn delete_hook(
    ctx: &IntegrationContext,
    connection: VcsConnectionId,
    project: &str,
    hook: &str,
) {
    let deleted = match credentials::gitlab_token(ctx, connection).await {
        Ok(token) => gitlab::delete_hook(&ctx.clients, &token, project, hook)
            .await
            .map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    };
    if let Err(error) = deleted {
        tracing::warn!(%connection, %project, %error, "a GitLab webhook stays");
    }
}
