//! The commit of a ref on GitHub: `GET /repos/{owner}/{repo}/commits/{ref}`, with an installation
//! token narrowed to the repository and to `contents: read`.

use scylla_domain::integrations::sources::GitRef;
use scylla_extension::Rejection;
use serde::Deserialize;

use super::commits::{Commit, Failure};
use super::store::Target;
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::{Auth, ProviderError, TokenScope};
use crate::Error;

/// The answer of `GET /repos/{owner}/{repo}/commits/{ref}`.
#[derive(Deserialize)]
struct Answer {
    /// The full SHA.
    sha: String,
    /// The Git data of the commit.
    commit: Detail,
    /// The GitHub account of the author, if GitHub knows one.
    author: Option<Author>,
}

/// The Git data of a commit.
#[derive(Deserialize)]
struct Detail {
    /// The message.
    message: String,
}

/// The GitHub account of the author of a commit.
#[derive(Deserialize)]
struct Author {
    /// The login.
    login: String,
    /// The address of the avatar.
    avatar_url: Option<String>,
}

/// Returns the commit of a ref of the repository of a GitHub connection.
pub(super) async fn commit(
    ctx: &IntegrationContext,
    target: &Target,
    git_ref: &GitRef,
) -> Result<Commit, Failure> {
    // Take the App of the connection: the App of the settings.
    let app = match (target.app, &ctx.clients.github_app) {
        (None, Some(app)) => app,
        _ => {
            let message = "the server has no GitHub App for the connection";
            return Err(Error::Rejected(Rejection::new("APP_MISSING", message)).into());
        }
    };

    // Get a token of the installation for the one repository.
    let installation = number(&target.account_id, "installation")?;
    let repository = number(&target.repository_id, "repository")?;
    let scope = TokenScope::default()
        .repository(repository)
        .permission("contents", "read");
    let token = app.installation_token(installation, &scope).await?;

    // Read the commit.
    let path = format!("/repos/{}/commits/{}", target.full_name, path_of(git_ref));
    let call = ctx
        .clients
        .github
        .get(&path)
        .auth(Auth::Bearer(&token.token));
    let answer: Answer = call.read().await.inspect_err(|error| {
        if matches!(error, ProviderError::Unauthorized) {
            app.forget(installation);
        }
    })?;
    Ok(Commit {
        sha: answer.sha,
        message: answer.commit.message,
        author_login: answer.author.as_ref().map(|author| author.login.clone()),
        avatar: answer.author.and_then(|author| author.avatar_url),
    })
}

/// Returns the ref of the commits route: `heads/<branch>`, `tags/<tag>` or the SHA, with each
/// part of a name encoded.
fn path_of(git_ref: &GitRef) -> String {
    let encoded = |name: &str| {
        name.split('/')
            .map(|part| url::form_urlencoded::byte_serialize(part.as_bytes()).collect::<String>())
            .collect::<Vec<_>>()
            .join("/")
    };
    match git_ref {
        GitRef::Branch(name) => format!("heads/{}", encoded(name)),
        GitRef::Tag(name) => format!("tags/{}", encoded(name)),
        GitRef::Sha(sha) => sha.clone(),
    }
}

/// Reads a GitHub id of a row.
fn number(text: &str, what: &str) -> Result<u64, Failure> {
    let error = || Error::Internal(format!("the {what} id {text:?} is not a number"));
    text.parse().map_err(|_| error().into())
}
