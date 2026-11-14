//! The sync of the file pipelines of a project with the files of its default branch: a new key
//! gives a new pipeline, a changed file changes its pipeline, and a file that left the branch
//! marks its pipeline as removed. The jobs stay.

use std::time::Duration;

use scylla_domain::identity::actors::ActorId;
use scylla_domain::integrations::links::PipelineSource;
use scylla_domain::projects::pipeline_files::{FileKey, Rejected};
use scylla_domain::projects::ProjectId;

use super::cache;
use super::read::{self, ReadFile, Snapshot};
use super::store::{self, FileRow};
use crate::features::integrations::IntegrationContext;
use crate::features::projects::pipelines::json;
use crate::Error;

/// The shortest time between two checks of the head of the default branch of a project.
const CHECK_EVERY: Duration = Duration::from_secs(30);

/// Reads the files of the default branch of a project of files again when its head changed, at
/// most each 30 seconds. A failure goes to the log only.
pub(crate) async fn refresh(ctx: &IntegrationContext, project: ProjectId) {
    if !cache::due(project, CHECK_EVERY) {
        return;
    }
    if let Err(error) = sync_default_branch(ctx, project, None).await {
        tracing::warn!(%project, %error, "the pipeline files of the default branch were not read");
    }
}

/// Makes the file pipelines of a project of files match the files at the head of its default
/// branch. `head` is that head when the caller knows it, else the provider gives it. A project
/// of the editor, and a head that the last sync read, change nothing.
pub(crate) async fn sync_default_branch(
    ctx: &IntegrationContext,
    project: ProjectId,
    head: Option<&str>,
) -> Result<(), Error> {
    // Stop for a project of the editor, and for a head that the last sync read.
    let linked = match read::link(ctx, project).await {
        Ok(linked) if linked.link.pipeline_source == PipelineSource::File => linked,
        Ok(_) | Err(Error::Rejected(_)) => return Ok(()),
        Err(error) => return Err(error),
    };
    let head = match head {
        Some(head) => head.to_owned(),
        None => read::head(ctx, &linked).await?,
    };
    if cache::head(project).as_deref() == Some(head.as_str()) {
        return Ok(());
    }

    // Read the files, then write the pipelines while the project still uses the files.
    let author = linked.link.created_by;
    let snapshot = read::read_commit(ctx, linked, &head).await?;
    let mut tx = ctx.pool.begin().await?;
    if store::lock_source(&mut tx, project).await? != Some(PipelineSource::File) {
        return Ok(());
    }
    apply(&mut tx, project, author, &snapshot).await?;
    tx.commit().await?;
    cache::saw(project, &head);
    Ok(())
}

/// Writes the file pipelines of a project from the files of a commit, in the transaction of the
/// caller. A new pipeline has `author` as its author.
pub(super) async fn apply(
    tx: &mut scylla_extension::Tx<'_>,
    project: ProjectId,
    author: ActorId,
    snapshot: &Snapshot,
) -> Result<(), Error> {
    // Write the pipeline of each file that the lookup takes.
    let mut keys = Vec::new();
    for read in &snapshot.files {
        store::upsert(tx, project, author, &row_of(read)?).await?;
        keys.push(read.file.key.as_str().to_owned());
    }

    // Write a pipeline with its error for each refused file of a key.
    for rejected in &snapshot.rejected {
        let Some(key) = &rejected.key else {
            continue;
        };
        store::upsert(tx, project, author, &refused_row(key, rejected)?).await?;
        keys.push(key.as_str().to_owned());
    }

    // Mark the pipelines of the other keys as removed.
    store::mark_removed(tx, project, &keys).await?;
    Ok(())
}

/// Returns the row of the pipeline of a file that the lookup takes: its nodes, its rules and its
/// checkout, or its errors and no node.
pub(super) fn row_of(read: &ReadFile) -> Result<FileRow<'_>, Error> {
    let key = read.file.key.as_str();
    let (name, nodes, rules, checkout, errors) = match &read.pipeline {
        Ok(pipeline) => (
            pipeline.name.as_str(),
            json::nodes(pipeline.nodes.as_slice()),
            Some(to_json(&pipeline.rules)?),
            Some(to_json(&pipeline.checkout)?),
            None,
        ),
        Err(errors) => (key, json::nodes(&[]), None, None, Some(to_json(errors)?)),
    };
    Ok(FileRow {
        key,
        path: &read.file.path,
        blob_sha: Some(&read.blob_sha),
        name,
        nodes,
        rules,
        checkout,
        errors,
    })
}

/// Returns the row of the pipeline of a key whose file the lookup refuses: the error and no node.
pub(super) fn refused_row<'a>(
    key: &'a FileKey,
    rejected: &'a Rejected,
) -> Result<FileRow<'a>, Error> {
    Ok(FileRow {
        key: key.as_str(),
        path: &rejected.error.path,
        blob_sha: None,
        name: key.as_str(),
        nodes: json::nodes(&[]),
        rules: None,
        checkout: None,
        errors: Some(to_json(&[&rejected.error])?),
    })
}

/// Returns the JSON of a value that the store keeps.
fn to_json<T: serde::Serialize + ?Sized>(value: &T) -> Result<serde_json::Value, Error> {
    serde_json::to_value(value).map_err(|error| Error::Internal(format!("a JSON value: {error}")))
}
