//! The read of the pipeline files of the linked repository at a commit: the folders and the blobs
//! (from the memory when the server read them before), the lookup, and the parse of each file.

use std::sync::Arc;

use scylla_domain::integrations::sources::GitRef;
use scylla_domain::projects::pipeline_files::{
    self, FileError, FilePipeline, FileRef, Rejected, TreeEntry,
};
use scylla_domain::projects::ProjectId;
use scylla_extension::Rejection;

use super::cache;
use super::repo::{Entry, Repo};
use crate::features::integrations::links::store::{self as links, Linked};
use crate::features::integrations::{sources, IntegrationContext};
use crate::Error;

/// The pipeline files of a commit, as the server read them.
pub(crate) struct Snapshot {
    /// The commit.
    pub(crate) commit_sha: String,
    /// The files that the lookup takes, in the order of their keys.
    pub(crate) files: Vec<ReadFile>,
    /// The files that the lookup refuses.
    pub(crate) rejected: Vec<Rejected>,
}

/// A pipeline file at a commit, with its text and its pipeline or its errors.
pub(crate) struct ReadFile {
    /// The key, the path and the format.
    pub(crate) file: FileRef,
    /// The SHA of the blob.
    pub(crate) blob_sha: String,
    /// The page of the file at the commit on the provider.
    pub(crate) url: String,
    /// The text of the file. Empty when the file is not UTF-8.
    pub(crate) text: String,
    /// The pipeline of the file, or each error that stops it.
    pub(crate) pipeline: Result<FilePipeline, Vec<FileError>>,
}

/// Reads the pipeline files of the linked repository of a project at a ref, or at the head of
/// the default branch with no ref. No link gives `NO_REPOSITORY_LINK`, and an unknown ref gives
/// `NOT_FOUND`.
pub(crate) async fn read_at(
    ctx: &IntegrationContext,
    project: ProjectId,
    git_ref: Option<GitRef>,
) -> Result<Snapshot, Error> {
    let linked = link(ctx, project).await?;
    let commit = match git_ref {
        Some(git_ref) => commit(ctx, project, &git_ref).await?,
        None => head(ctx, &linked).await?,
    };
    read_commit(ctx, linked, &commit).await
}

/// Returns the link of a project. No link gives `NO_REPOSITORY_LINK`.
pub(super) async fn link(ctx: &IntegrationContext, project: ProjectId) -> Result<Linked, Error> {
    let mut tx = ctx.pool.begin().await?;
    let linked = links::find(&mut tx, project).await?;
    tx.commit().await?;
    linked.ok_or_else(|| {
        let message = "the project has no linked repository";
        Error::Rejected(Rejection::new("NO_REPOSITORY_LINK", message))
    })
}

/// Returns the SHA of the head of the default branch of a link.
pub(super) async fn head(ctx: &IntegrationContext, linked: &Linked) -> Result<String, Error> {
    let branch = GitRef::branch(linked.link.default_branch.clone())
        .map_err(|error| Error::Internal(format!("the default branch {error}")))?;
    commit(ctx, linked.link.project_id, &branch).await
}

/// Returns the full SHA of the commit of a ref.
async fn commit(
    ctx: &IntegrationContext,
    project: ProjectId,
    git_ref: &GitRef,
) -> Result<String, Error> {
    Ok(sources::resolve(ctx, project, git_ref).await?.sha)
}

/// Reads the pipeline files of the repository of a link at a commit.
pub(super) async fn read_commit(
    ctx: &IntegrationContext,
    linked: Linked,
    commit: &str,
) -> Result<Snapshot, Error> {
    // List the folders, from the memory when the server read them before, and find the files.
    let project = linked.link.project_id;
    let repo = Repo::open(ctx, linked).await?;
    let entries = match cache::tree(project, commit) {
        Some(entries) => entries,
        None => {
            let entries: Arc<[Entry]> = repo.entries(ctx, commit).await?.into();
            cache::keep_tree(project, commit, Arc::clone(&entries));
            entries
        }
    };
    let tree: Vec<TreeEntry> = entries
        .iter()
        .map(|entry| TreeEntry {
            path: entry.path.clone(),
            size: entry.size,
        })
        .collect();
    let found = pipeline_files::lookup(&tree);

    // Read and parse each file, with the blobs that the server read before.
    let mut files = Vec::with_capacity(found.files.len());
    for file in found.files {
        let blob_sha = entries
            .iter()
            .find(|entry| entry.path == file.path)
            .map(|entry| entry.blob_sha.clone())
            .unwrap_or_default();
        let bytes = match cache::blob(project, &blob_sha) {
            Some(bytes) => bytes,
            None => {
                let bytes: Arc<[u8]> = repo.blob(ctx, &blob_sha).await?.into();
                cache::keep(project, &blob_sha, Arc::clone(&bytes));
                bytes
            }
        };
        let (text, pipeline) = match std::str::from_utf8(&bytes) {
            Ok(text) => {
                let parsed = pipeline_files::parse(&file.path, &file.key, file.format, text);
                (text.to_owned(), parsed)
            }
            Err(_) => {
                let error = FileError::of_file(&file.path, "the file is not UTF-8 text");
                (String::new(), Err(vec![error]))
            }
        };
        files.push(ReadFile {
            url: repo.page_of(commit, &file.path),
            file,
            blob_sha,
            text,
            pipeline,
        });
    }
    Ok(Snapshot {
        commit_sha: commit.to_owned(),
        files,
        rejected: found.rejected,
    })
}
