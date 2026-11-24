//! The commit of a ref on GitHub (`GET /repos/{owner}/{repo}/commits/{ref}`, with an installation
//! token narrowed to the repository and to `contents: read`), and the files of a pull request
//! (`GET /repos/{owner}/{repo}/pulls/{number}/files`, with `pull_requests: read`).

use std::sync::Arc;

use scylla_domain::integrations::sources::GitRef;
use serde::Deserialize;

use super::commits::{Commit, Failure};
use super::store::Target;
use crate::features::integrations::connections::credentials;
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::{Auth, GitHubApp, Hidden, ProviderError, TokenScope};
use crate::Error;

/// The most files of one page of GitHub.
const PAGE: u32 = 100;

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

/// A file of a pull request.
#[derive(Deserialize)]
struct ChangedFile {
    /// The path of the file.
    filename: String,
    /// The old path of a renamed file.
    previous_filename: Option<String>,
}

/// An installation token of the App of a connection, narrowed to one repository.
struct Token {
    /// The App, which forgets the token after a 401.
    app: Arc<GitHubApp>,
    /// The installation.
    installation: u64,
    /// The token.
    token: Hidden,
}

/// Returns the commit of a ref of the repository of a GitHub connection.
pub(super) async fn commit(
    ctx: &IntegrationContext,
    target: &Target,
    git_ref: &GitRef,
) -> Result<Commit, Failure> {
    let token = token(ctx, target, "contents").await?;
    let path = format!("/repos/{}/commits/{}", target.full_name, path_of(git_ref));
    let call = ctx
        .clients
        .github
        .get(&path)
        .auth(Auth::Bearer(&token.token));
    let answer: Answer = read(&token, call.read()).await?;
    Ok(Commit {
        sha: answer.sha,
        message: answer.commit.message,
        author_login: answer.author.as_ref().map(|author| author.login.clone()),
        avatar: answer.author.and_then(|author| author.avatar_url),
    })
}

/// Returns the paths that a pull request of the repository of a GitHub connection changes, with
/// the old path of each renamed file.
pub(super) async fn changed_paths(
    ctx: &IntegrationContext,
    target: &Target,
    number: u32,
) -> Result<Vec<String>, Failure> {
    let token = token(ctx, target, "pull_requests").await?;
    let path = format!("/repos/{}/pulls/{number}/files", target.full_name);
    let call = ctx
        .clients
        .github
        .get(&path)
        .auth(Auth::Bearer(&token.token))
        .query("per_page", PAGE);
    let files: Vec<ChangedFile> = read(&token, call.list(None)).await?;
    Ok(files
        .into_iter()
        .flat_map(|file| std::iter::once(file.filename).chain(file.previous_filename))
        .collect())
}

/// Returns a token of the installation of a connection for its repository, with the `read`
/// access of one permission.
async fn token(
    ctx: &IntegrationContext,
    target: &Target,
    permission: &str,
) -> Result<Token, Failure> {
    // Take the App of the connection: the App of the settings, else the App of the instance.
    let source = credentials::github_app(ctx).await?;
    let app = match source {
        Some(source) if target.app.is_none() || source.row == target.app => source.app,
        _ => return Err(credentials::app_missing().into()),
    };

    // Get a token of the installation for the one repository.
    let installation = number(&target.account_id, "installation")?;
    let repository = number(&target.repository_id, "repository")?;
    let scope = TokenScope::default()
        .repository(repository)
        .permission(permission, "read");
    let token = app.installation_token(installation, &scope).await?.token;
    Ok(Token {
        app,
        installation,
        token,
    })
}

/// Waits for a call with a token, and makes the App forget the token after a 401.
async fn read<T>(
    token: &Token,
    call: impl std::future::Future<Output = Result<T, ProviderError>>,
) -> Result<T, ProviderError> {
    call.await.inspect_err(|error| {
        if matches!(error, ProviderError::Unauthorized) {
            token.app.forget(token.installation);
        }
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
