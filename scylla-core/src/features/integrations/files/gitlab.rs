//! The calls of the pipeline files to GitLab: the tree of `.scylla` at a commit
//! (`GET /projects/:id/repository/tree?path=.scylla&recursive=true`), and a blob
//! (`GET /projects/:id/repository/blobs/:sha`).

use serde::Deserialize;

use super::repo::{Blob, Entry};
use crate::infra::vcs::{Auth, Hidden, ProviderError, VcsClients};

/// The most items of one page of GitLab.
const PAGE: u32 = 100;

/// An item of a tree.
#[derive(Deserialize)]
struct Item {
    /// The SHA of the blob of a file.
    id: String,
    /// The path from the root of the repository.
    path: String,
    /// `blob`, `tree` or `commit`.
    #[serde(rename = "type")]
    kind: String,
}

/// The answer of a blob.
#[derive(Deserialize)]
struct BlobAnswer {
    /// The bytes, in the encoding.
    content: String,
    /// The encoding: `base64`.
    encoding: String,
}

/// Lists the files below `.scylla` at a commit. No folder gives no file. GitLab gives no size.
pub(super) async fn entries(
    clients: &VcsClients,
    token: &Hidden,
    project: &str,
    commit: &str,
) -> Result<Vec<Entry>, ProviderError> {
    let target = format!("/projects/{project}/repository/tree");
    let listed = clients
        .gitlab
        .get(&target)
        .auth(Auth::PrivateToken(token))
        .query("path", ".scylla")
        .query("ref", commit)
        .query("recursive", "true")
        .query("per_page", PAGE)
        .list::<Item>(None)
        .await;
    let items = match listed {
        Ok(items) => items,
        Err(ProviderError::NotFound) => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    Ok(items
        .into_iter()
        .filter(|item| item.kind == "blob")
        .map(|item| Entry {
            path: item.path,
            blob_sha: item.id,
            size: None,
        })
        .collect())
}

/// Reads a blob of a project.
pub(super) async fn blob(
    clients: &VcsClients,
    token: &Hidden,
    project: &str,
    sha: &str,
) -> Result<Blob, ProviderError> {
    let target = format!("/projects/{project}/repository/blobs/{sha}");
    let answer: BlobAnswer = clients
        .gitlab
        .get(&target)
        .auth(Auth::PrivateToken(token))
        .read()
        .await?;
    Ok(Blob {
        content: answer.content,
        encoding: answer.encoding,
    })
}
