//! The commit of a ref on GitLab (`GET /projects/:id/repository/commits/:ref`) and the files of a
//! merge request (`GET /projects/:id/merge_requests/:iid/diffs`), with the token of the service
//! account of the connection.

use scylla_domain::integrations::sources::GitRef;
use scylla_extension::Rejection;
use serde::Deserialize;

use super::commits::{Commit, Failure};
use super::store::Target;
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::{Auth, Hidden};
use crate::Error;

/// The most files of one page of GitLab.
const PAGE: u32 = 100;

/// The answer of `GET /projects/:id/repository/commits/:ref`.
#[derive(Deserialize)]
struct Answer {
    /// The full SHA.
    id: String,
    /// The message.
    message: String,
}

/// A file of a merge request.
#[derive(Deserialize)]
struct Diff {
    /// The path before the change.
    old_path: String,
    /// The path after the change.
    new_path: String,
}

/// Returns the commit of a ref of the repository of a GitLab connection. GitLab gives no login
/// of the author of a commit.
pub(super) async fn commit(
    ctx: &IntegrationContext,
    target: &Target,
    git_ref: &GitRef,
) -> Result<Commit, Failure> {
    let token = token(ctx, target)?;
    let name = match git_ref {
        GitRef::Branch(name) | GitRef::Tag(name) | GitRef::Sha(name) => name,
    };
    let encoded: String = url::form_urlencoded::byte_serialize(name.as_bytes()).collect();
    let path = format!(
        "/projects/{}/repository/commits/{encoded}",
        target.repository_id
    );
    let call = ctx
        .clients
        .gitlab
        .get(&path)
        .auth(Auth::PrivateToken(&token));
    let answer: Answer = call.read().await?;
    Ok(Commit {
        sha: answer.id,
        message: answer.message,
        author_login: None,
        avatar: None,
    })
}

/// Returns the paths that a merge request of the repository of a GitLab connection changes:
/// the old and the new path of each file.
pub(super) async fn changed_paths(
    ctx: &IntegrationContext,
    target: &Target,
    number: u32,
) -> Result<Vec<String>, Failure> {
    let token = token(ctx, target)?;
    let path = format!(
        "/projects/{}/merge_requests/{number}/diffs",
        target.repository_id
    );
    let call = ctx
        .clients
        .gitlab
        .get(&path)
        .auth(Auth::PrivateToken(&token))
        .query("per_page", PAGE);
    let diffs: Vec<Diff> = call.list(None).await?;
    Ok(diffs
        .into_iter()
        .flat_map(|diff| [diff.old_path, diff.new_path])
        .collect())
}

/// Opens the token of the service account of a connection.
fn token(ctx: &IntegrationContext, target: &Target) -> Result<Hidden, Failure> {
    let Some(sealer) = ctx.sealer.as_deref() else {
        let message = "the server has no key of the secrets";
        return Err(Error::Rejected(Rejection::new("SECRET_KEY_MISSING", message)).into());
    };
    let connection = target.connection;
    let broken = |reason: &str| Error::Internal(format!("the token of {connection}: {reason}"));
    let sealed = target
        .sealed_token
        .as_deref()
        .ok_or_else(|| broken("missing"))?;
    let plain = sealer
        .open_with(sealed, &connection.to_bytes())
        .map_err(|reason| broken(&reason))?;
    let token = String::from_utf8(plain).map_err(|_| broken("not UTF-8"))?;
    Ok(Hidden::new(token))
}
