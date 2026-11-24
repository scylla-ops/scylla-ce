//! The heads of a GitHub repository for a poll: its branches and its open pull requests with the
//! ETag of each list (a 304 answer does not count against the limit), and the commit of a new
//! head. The calls use an installation token narrowed to the repository, `contents: read` and
//! `pull_requests: read`.

use scylla_domain::integrations::events::EventCommit;
use scylla_domain::integrations::sources::PullRequestRef;
use serde::de::DeserializeOwned;
use serde::Deserialize;

use super::listed::{Cursor, Head, HeadKind, Listed};
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::{Answer, Auth, GitHubApp, Hidden, ProviderError, TokenScope};

/// The most items of one page of GitHub.
const PAGE: u32 = 100;

/// The repository of a poll: the App and the installation that see it, its id and its name.
pub(super) struct Repository<'a> {
    /// The App of the connection.
    pub(super) app: &'a GitHubApp,
    /// The id of the installation.
    pub(super) installation: u64,
    /// The id of the repository.
    pub(super) id: u64,
    /// The full name, for example `acme/api`.
    pub(super) full_name: &'a str,
}

/// A branch of a list.
#[derive(Deserialize)]
struct Branch {
    /// The name.
    name: String,
    /// The head commit.
    commit: Sha,
}

/// A commit with only its SHA.
#[derive(Deserialize)]
struct Sha {
    /// The SHA.
    sha: String,
}

/// An open pull request of a list.
#[derive(Deserialize)]
struct Pull {
    /// The number.
    number: u32,
    /// The title.
    title: String,
    /// The page.
    html_url: String,
    /// `true` for a draft.
    #[serde(default)]
    draft: bool,
    /// The source branch.
    head: Side,
    /// The target branch.
    base: Side,
    /// The author.
    user: Option<User>,
}

/// A side of a pull request.
#[derive(Deserialize)]
struct Side {
    /// The branch.
    #[serde(rename = "ref")]
    name: String,
    /// The commit.
    sha: String,
    /// The repository, or `None` when the fork was deleted.
    repo: Option<Repo>,
}

/// The repository of a side.
#[derive(Deserialize)]
struct Repo {
    /// Its id.
    id: u64,
    /// Its full name.
    full_name: String,
}

/// An account of GitHub.
#[derive(Deserialize)]
struct User {
    /// The login.
    login: String,
    /// The address of the avatar.
    avatar_url: Option<String>,
}

/// A commit, from `GET /repos/{owner}/{repo}/commits/{sha}`.
#[derive(Deserialize)]
struct Commit {
    /// The Git data.
    commit: Detail,
    /// The GitHub account of the author.
    author: Option<User>,
}

/// The Git data of a commit.
#[derive(Deserialize)]
struct Detail {
    /// The message.
    message: String,
}

/// Reads the branches and the open pull requests of a repository. A list whose ETag did not
/// change gives `None`.
pub(super) async fn list(
    ctx: &IntegrationContext,
    repository: &Repository<'_>,
    branches_etag: Option<&str>,
    pulls_etag: Option<&str>,
) -> Result<Listed, ProviderError> {
    // Read the branches and the open pull requests.
    let token = token(repository).await?;
    let path = format!("/repos/{}/branches", repository.full_name);
    let (branches, branches_tag) = read::<Branch>(ctx, &token, &path, branches_etag, false).await?;
    let path = format!("/repos/{}/pulls", repository.full_name);
    let (pulls, pulls_tag) = read::<Pull>(ctx, &token, &path, pulls_etag, true).await?;

    // Make the heads, and the cursors of the next poll.
    let branch = |branch: Branch| Head {
        kind: HeadKind::Branch(branch.name),
        commit: Head::commit_of(branch.commit.sha),
    };
    Ok(Listed {
        branches: branches.map(|found| found.into_iter().map(branch).collect()),
        branches_cursor: Cursor {
            etag: branches_tag,
            updated_after: None,
        },
        pulls: pulls.map(|found| found.into_iter().map(pull).collect()),
        all_pulls: true,
        pulls_cursor: Cursor {
            etag: pulls_tag,
            updated_after: None,
        },
    })
}

/// Returns the commit of a new head of a branch: its message and its author.
pub(super) async fn commit(
    ctx: &IntegrationContext,
    repository: &Repository<'_>,
    sha: &str,
) -> Result<EventCommit, ProviderError> {
    let token = token(repository).await?;
    let path = format!("/repos/{}/commits/{sha}", repository.full_name);
    let call = ctx.clients.github.get(&path).auth(Auth::Bearer(&token));
    let found: Commit = call.read().await?;
    Ok(EventCommit {
        sha: sha.to_owned(),
        message: Some(found.commit.message),
        author_login: found.author.as_ref().map(|author| author.login.clone()),
        avatar: found.author.and_then(|author| author.avatar_url),
    })
}

/// Returns an installation token of the repository. A refused token leaves the cache.
async fn token(repository: &Repository<'_>) -> Result<Hidden, ProviderError> {
    let scope = TokenScope::default()
        .repository(repository.id)
        .permission("contents", "read")
        .permission("pull_requests", "read");
    let token = repository
        .app
        .installation_token(repository.installation, &scope)
        .await;
    if matches!(token, Err(ProviderError::Unauthorized)) {
        repository.app.forget(repository.installation);
    }
    Ok(token?.token)
}

/// Reads each page of a list, with the ETag of its first page. A first page that did not change
/// gives `None` and the same ETag.
async fn read<T: DeserializeOwned>(
    ctx: &IntegrationContext,
    token: &Hidden,
    path: &str,
    etag: Option<&str>,
    open: bool,
) -> Result<(Option<Vec<T>>, Option<String>), ProviderError> {
    let mut call = ctx
        .clients
        .github
        .get(path)
        .auth(Auth::Bearer(token))
        .query("per_page", PAGE)
        .if_none_match(etag);
    if open {
        call = call.query("state", "open");
    }
    match call.send::<Vec<T>>().await? {
        Answer::NotModified => Ok((None, etag.map(str::to_owned))),
        Answer::Fresh {
            mut body,
            etag,
            next,
        } => {
            if let Some(next) = next {
                let rest = ctx.clients.github.get(&next).auth(Auth::Bearer(token));
                body.extend(rest.list::<T>(None).await?);
            }
            Ok((Some(body), etag))
        }
    }
}

/// Returns the head of an open pull request. A source branch of another repository is a fork.
fn pull(pull: Pull) -> Head {
    let fork = match (&pull.head.repo, &pull.base.repo) {
        (Some(head), Some(base)) if head.id == base.id => None,
        (Some(head), _) => Some(head.full_name.clone()),
        (None, _) => Some(String::new()),
    };
    Head {
        kind: HeadKind::Pull {
            pull: PullRequestRef {
                number: pull.number,
                title: pull.title,
                url: pull.html_url,
                source_branch: pull.head.name,
                target_branch: pull.base.name,
            },
            draft: pull.draft,
            fork,
        },
        commit: EventCommit {
            sha: pull.head.sha,
            message: None,
            author_login: pull.user.as_ref().map(|user| user.login.clone()),
            avatar: pull.user.and_then(|user| user.avatar_url),
        },
    }
}
