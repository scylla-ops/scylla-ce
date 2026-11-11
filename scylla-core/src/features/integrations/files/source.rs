//! The source of the pipelines of a project: the editor of Scylla, or the files of the linked
//! repository. One source at a time: the pipelines of the other source stop, and their jobs stay.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::integrations::links::PipelineSource;
use scylla_domain::projects::ProjectId;
use scylla_extension::Rejection;

use super::{cache, read, store, sync};
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// Sets the source of the pipelines of a project. To the files: the project needs a linked
/// repository, else `NO_REPOSITORY_LINK`; the server reads the files of the default branch, then
/// archives the editor pipelines and makes the file pipelines in one transaction. To the editor:
/// the archived pipelines come back, and the file pipelines are hidden.
pub(crate) async fn set_source(
    ctx: &IntegrationContext,
    actor: ActorId,
    project: ProjectId,
    source: PipelineSource,
) -> Result<(), Error> {
    match source {
        PipelineSource::File => to_files(ctx, actor, project).await,
        PipelineSource::Editor => to_editor(ctx, project).await,
    }
}

/// Moves a project to the files of the default branch of its repository.
async fn to_files(
    ctx: &IntegrationContext,
    actor: ActorId,
    project: ProjectId,
) -> Result<(), Error> {
    // Read the files of the default branch.
    let linked = read::link(ctx, project).await?;
    let head = read::head(ctx, &linked).await?;
    let snapshot = read::read_commit(ctx, linked, &head).await?;

    // Archive the editor pipelines and write the file pipelines in one transaction.
    let mut tx = ctx.pool.begin().await?;
    if store::lock_source(&mut tx, project).await?.is_none() {
        let message = "the project has no linked repository";
        return Err(Error::Rejected(Rejection::new(
            "NO_REPOSITORY_LINK",
            message,
        )));
    }
    store::set_source(&mut tx, project, PipelineSource::File).await?;
    let archived = store::archive_editor(&mut tx, project).await?;
    sync::apply(&mut tx, project, actor, &snapshot).await?;
    tx.commit().await?;
    cache::saw(project, &head);
    tracing::info!(
        event = "pipeline_source.changed",
        %project,
        source = PipelineSource::File.as_str(),
        commit = %head,
        files = snapshot.files.len(),
        archived,
    );
    Ok(())
}

/// Moves a project back to the editor. A project with no link can do it too.
async fn to_editor(ctx: &IntegrationContext, project: ProjectId) -> Result<(), Error> {
    let mut tx = ctx.pool.begin().await?;
    if store::lock_source(&mut tx, project).await?.is_some() {
        store::set_source(&mut tx, project, PipelineSource::Editor).await?;
    }
    store::restore_editor(&mut tx, project).await?;
    tx.commit().await?;
    tracing::info!(
        event = "pipeline_source.changed",
        %project,
        source = PipelineSource::Editor.as_str(),
    );
    Ok(())
}
