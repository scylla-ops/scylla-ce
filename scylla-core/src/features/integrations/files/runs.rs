//! The runs of the file pipelines: the job of a key starts from the file of its key at the commit
//! of the job, with the nodes and the checkout of the file, or it fails at once with the errors of
//! the file. A Git event starts each key whose `on` table matches it.

use scylla_domain::integrations::sources::{GitRef, JobSource, SourceEvent};
use scylla_domain::projects::git_rules::TriggerEvent;
use scylla_domain::projects::jobs::{PipelineFileRef, StartJob};
use scylla_domain::projects::pipeline_files::{CheckoutSettings, FileError, FilePipeline};
use scylla_domain::projects::pipelines::{Pipeline, PipelineId};
use scylla_domain::projects::ProjectId;

use super::read::{self, Snapshot};
use super::{repo, store, sync};
use crate::features::integrations::links::store::Linked;
use crate::features::integrations::{sources, IntegrationContext};
use crate::features::projects::pipelines::service as pipelines;
use crate::Error;

/// The file of a key at a commit.
struct KeyFile {
    /// The file in the repository at the commit.
    file: PipelineFileRef,
    /// The pipeline of the file, or each error that stops it.
    pipeline: Result<FilePipeline, Vec<FileError>>,
}

/// Returns the source of a run at the head of the default branch of the linked repository of a
/// project, with an event. No link gives `NO_REPOSITORY_LINK`.
pub(crate) async fn head_source(
    ctx: &IntegrationContext,
    project: ProjectId,
    event: SourceEvent,
) -> Result<JobSource, Error> {
    let linked = read::link(ctx, project).await?;
    commit_of(ctx, &linked, None, event).await
}

/// Makes the start of a job of a file pipeline from the file of its key at a ref, or at the head
/// of the default branch with no ref. The source of the job has the event. A commit with no file
/// of the key gives `NOT_FOUND`.
pub(crate) async fn start_at(
    ctx: &IntegrationContext,
    pipeline: Pipeline,
    git_ref: Option<GitRef>,
    event: SourceEvent,
) -> Result<StartJob, Error> {
    // Find the commit of the ref.
    let key = pipeline.file.as_ref().map(|file| file.key.clone());
    let key = key.ok_or_else(|| Error::Internal("a run of a file needs a file pipeline".into()))?;
    let linked = read::link(ctx, pipeline.project_id).await?;
    let source = commit_of(ctx, &linked, git_ref, event).await?;

    // Read the files at the commit, and take the file of the key.
    let snapshot = read::read_commit(ctx, linked.clone(), &source.sha).await?;
    let file = file_of(&linked, &snapshot, &key).ok_or(Error::NotFound("pipeline file"))?;
    Ok(start(pipeline, file, source))
}

/// Makes the starts of the jobs of a Git event in a project of files, from the files at the commit
/// of its source: each key whose `on` table matches the event. On a push or a pull request, each
/// key whose file has errors also starts, because the server cannot read its `on` table. A key
/// with no pipeline gets one that is not on the default branch.
pub(crate) async fn starts_of_event(
    ctx: &IntegrationContext,
    linked: &Linked,
    event: &TriggerEvent<'_>,
    source: &JobSource,
) -> Result<Vec<StartJob>, Error> {
    // Read the files at the commit, and find the keys that the event starts.
    let snapshot = read::read_commit(ctx, linked.clone(), &source.sha).await?;
    let invalid_runs = !matches!(event, TriggerEvent::Tag { .. });
    let matched = snapshot.files.iter().filter(|read| match &read.pipeline {
        Ok(pipeline) => pipeline.rules.matches(event),
        Err(_) => invalid_runs,
    });
    let mut keys: Vec<&str> = matched.map(|read| read.file.key.as_str()).collect();
    for rejected in snapshot.rejected.iter().filter(|_| invalid_runs) {
        let key = rejected.key.as_ref().map(|key| key.as_str());
        if let Some(key) = key.filter(|key| !keys.contains(key)) {
            keys.push(key);
        }
    }

    // Find or make the pipeline of each key, and make its start from its file.
    let mut starts = Vec::with_capacity(keys.len());
    for key in keys {
        let Some(file) = file_of(linked, &snapshot, key) else {
            continue;
        };
        let id = pipeline_of(ctx, linked, &snapshot, key).await?;
        let pipeline = pipelines::get(&ctx.pool, id).await?;
        let pipeline = pipeline.ok_or(Error::NotFound("pipeline"))?;
        starts.push(start(pipeline, file, source.clone()));
    }
    Ok(starts)
}

/// Returns the source of a run at a ref of the repository of a link, or at the head of its
/// default branch with no ref, with an event.
async fn commit_of(
    ctx: &IntegrationContext,
    linked: &Linked,
    git_ref: Option<GitRef>,
    event: SourceEvent,
) -> Result<JobSource, Error> {
    let git_ref = match git_ref {
        Some(git_ref) => git_ref,
        None => GitRef::branch(linked.link.default_branch.clone())
            .map_err(|error| Error::Internal(format!("the default branch {error}")))?,
    };
    let mut source = sources::resolve(ctx, linked.link.project_id, &git_ref).await?;
    source.event = event;
    Ok(source)
}

/// Returns the file pipeline of a key, and makes it when the project has none.
async fn pipeline_of(
    ctx: &IntegrationContext,
    linked: &Linked,
    snapshot: &Snapshot,
    key: &str,
) -> Result<PipelineId, Error> {
    // Make the row of the pipeline from the file of the key, or from its refusal.
    let read = snapshot
        .files
        .iter()
        .find(|read| read.file.key.as_str() == key);
    let row = match read {
        Some(read) => sync::row_of(read)?,
        None => {
            let refused = snapshot.rejected.iter().find_map(|rejected| {
                let found = rejected
                    .key
                    .as_ref()
                    .filter(|found| found.as_str() == key)?;
                Some((found, rejected))
            });
            let (found, rejected) = refused.ok_or(Error::NotFound("pipeline file"))?;
            sync::refused_row(found, rejected)?
        }
    };

    // Find the pipeline of the key, or make it.
    let project = linked.link.project_id;
    let mut tx = ctx.pool.begin().await?;
    let id = store::key_pipeline(&mut tx, project, linked.link.created_by, &row).await?;
    tx.commit().await?;
    Ok(id)
}

/// Returns the file of a key at the commit of the files: the file that the lookup takes, or the
/// errors of each refused file of the key. `None` when the commit has no file of the key.
fn file_of(linked: &Linked, snapshot: &Snapshot, key: &str) -> Option<KeyFile> {
    // Take the file that the lookup takes.
    let commit = &snapshot.commit_sha;
    if let Some(read) = snapshot
        .files
        .iter()
        .find(|read| read.file.key.as_str() == key)
    {
        return Some(KeyFile {
            file: PipelineFileRef {
                path: read.file.path.clone(),
                blob_sha: read.blob_sha.clone(),
                commit_sha: commit.clone(),
                url: read.url.clone(),
            },
            pipeline: read.pipeline.clone(),
        });
    }

    // Take the errors of the refused files of the key.
    let errors: Vec<FileError> = snapshot
        .rejected
        .iter()
        .filter(|rejected| {
            rejected
                .key
                .as_ref()
                .is_some_and(|found| found.as_str() == key)
        })
        .map(|rejected| rejected.error.clone())
        .collect();
    let path = errors.first()?.path.clone();
    Some(KeyFile {
        file: PipelineFileRef {
            url: repo::page(linked, commit, &path),
            path,
            blob_sha: String::new(),
            commit_sha: commit.clone(),
        },
        pipeline: Err(errors),
    })
}

/// Makes the start of a job of a file pipeline on a commit: the nodes and the checkout of its
/// file, or the errors of the file with no node and no checkout.
fn start(pipeline: Pipeline, file: KeyFile, source: JobSource) -> StartJob {
    let (nodes, checkout, file_errors) = match file.pipeline {
        Ok(found) => (found.nodes.into_vec(), found.checkout, Vec::new()),
        Err(errors) => {
            let off = CheckoutSettings {
                enabled: false,
                ..CheckoutSettings::default()
            };
            (Vec::new(), off, errors)
        }
    };
    StartJob {
        nodes,
        checkout,
        source: Some(source),
        pipeline_file: Some(file.file),
        file_errors,
        ..StartJob::of(pipeline)
    }
}
