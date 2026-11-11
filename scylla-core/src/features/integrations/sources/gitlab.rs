//! The commit of a ref on GitLab: `GET /projects/:id/repository/commits/:ref`, with the token of
//! the service account of the connection.

use scylla_domain::integrations::sources::GitRef;
use scylla_extension::Rejection;
use serde::Deserialize;

use super::commits::{Commit, Failure};
use super::store::Target;
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::{Auth, Hidden};
use crate::Error;

/// The answer of `GET /projects/:id/repository/commits/:ref`.
#[derive(Deserialize)]
struct Answer {
    /// The full SHA.
    id: String,
    /// The message.
    message: String,
}

/// Returns the commit of a ref of the repository of a GitLab connection. GitLab gives no login
/// of the author of a commit.
pub(super) async fn commit(
    ctx: &IntegrationContext,
    target: &Target,
    git_ref: &GitRef,
) -> Result<Commit, Failure> {
    // Open the token of the service account.
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
    let token = Hidden::new(String::from_utf8(plain).map_err(|_| broken("not UTF-8"))?);

    // Read the commit.
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
