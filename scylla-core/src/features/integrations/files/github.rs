//! The calls of the pipeline files to GitHub: the folders `.scylla` and `.scylla/pipelines` at a
//! commit (`GET /repos/{owner}/{repo}/contents/{path}?ref=`), and a blob
//! (`GET /repos/{owner}/{repo}/git/blobs/{sha}`).

use serde::Deserialize;
use serde_json::Value;

use super::repo::{Blob, Entry};
use crate::infra::vcs::{Auth, Hidden, ProviderError, VcsClients};

/// The folder of the pipeline files.
const ROOT: &str = ".scylla";

/// The folder of many pipeline files.
const FOLDER: &str = ".scylla/pipelines";

/// An item of a folder.
#[derive(Deserialize)]
struct Item {
    /// The path from the root of the repository.
    path: String,
    /// The SHA of the blob of a file.
    sha: String,
    /// The size in bytes.
    size: u64,
    /// `file`, `dir`, `symlink` or `submodule`.
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

/// Lists the files of `.scylla`, and of `.scylla/pipelines` when it is a folder, at a commit.
pub(super) async fn entries(
    clients: &VcsClients,
    token: &Hidden,
    full_name: &str,
    commit: &str,
) -> Result<Vec<Entry>, ProviderError> {
    // Read the root folder.
    let root = folder(clients, token, full_name, ROOT, commit).await?;
    let many = root
        .iter()
        .any(|item| item.path == FOLDER && item.kind == "dir");

    // Read the folder of many files, when it is there.
    let mut items = root;
    if many {
        items.extend(folder(clients, token, full_name, FOLDER, commit).await?);
    }
    Ok(items
        .into_iter()
        .filter(|item| item.kind == "file")
        .map(|item| Entry {
            path: item.path,
            blob_sha: item.sha,
            size: Some(item.size),
        })
        .collect())
}

/// Reads a blob of a repository.
pub(super) async fn blob(
    clients: &VcsClients,
    token: &Hidden,
    full_name: &str,
    sha: &str,
) -> Result<Blob, ProviderError> {
    let path = format!("/repos/{full_name}/git/blobs/{sha}");
    let answer: BlobAnswer = clients
        .github
        .get(&path)
        .auth(Auth::Bearer(token))
        .read()
        .await?;
    Ok(Blob {
        content: answer.content,
        encoding: answer.encoding,
    })
}

/// Lists the items of a folder at a commit. No folder, or a file at the path, gives no item.
async fn folder(
    clients: &VcsClients,
    token: &Hidden,
    full_name: &str,
    path: &str,
    commit: &str,
) -> Result<Vec<Item>, ProviderError> {
    let target = format!("/repos/{full_name}/contents/{path}");
    let call = clients
        .github
        .get(&target)
        .auth(Auth::Bearer(token))
        .query("ref", commit);
    let answer: Value = match call.read().await {
        Ok(answer) => answer,
        Err(ProviderError::NotFound) => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    if !answer.is_array() {
        return Ok(Vec::new());
    }
    serde_json::from_value(answer)
        .map_err(|error| ProviderError::Invalid(format!("a folder is not valid: {error}")))
}
