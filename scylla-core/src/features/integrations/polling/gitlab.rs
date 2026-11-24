//! The heads of a GitLab project for a poll, with the token of the service account: its branches,
//! its open merge requests that changed since the last poll (`updated_after`), and the state of
//! the webhook of the project.

use scylla_domain::integrations::events::EventCommit;
use scylla_domain::integrations::sources::PullRequestRef;
use serde::Deserialize;
use time::{Duration, OffsetDateTime};

use super::listed::{Cursor, Head, HeadKind, Listed};
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::{Auth, Call, Hidden, ProviderError};

/// The most items of one page of GitLab.
const PAGE: u32 = 100;

/// The time before the start of a poll from which the next poll asks for the changed merge
/// requests, against a drift of the clocks.
const OVERLAP: Duration = Duration::minutes(5);

/// The state of a webhook that GitLab calls.
const EXECUTABLE: &str = "executable";

/// A branch of a list.
#[derive(Deserialize)]
struct Branch {
    /// The name.
    name: String,
    /// The head commit.
    commit: Commit,
}

/// A commit of a branch.
#[derive(Deserialize)]
struct Commit {
    /// The SHA.
    id: String,
    /// The message.
    message: Option<String>,
}

/// An open merge request of a list.
#[derive(Deserialize)]
struct MergeRequest {
    /// The number in the project.
    iid: u32,
    /// The title.
    title: String,
    /// The page.
    web_url: String,
    /// The source branch.
    source_branch: String,
    /// The target branch.
    target_branch: String,
    /// The project of the source branch.
    source_project_id: u64,
    /// The project of the target branch.
    target_project_id: u64,
    /// The head commit.
    sha: Option<String>,
    /// `true` for a draft.
    #[serde(default)]
    draft: bool,
    /// The author.
    author: Option<User>,
}

/// A GitLab user.
#[derive(Deserialize)]
struct User {
    /// The login.
    username: String,
    /// The address of the avatar.
    avatar_url: Option<String>,
}

/// The source project of a merge request from a fork.
#[derive(Deserialize)]
struct Source {
    /// Its full path.
    path_with_namespace: String,
}

/// A webhook of a project.
#[derive(Deserialize)]
struct Hook {
    /// `executable`, `temporarily_disabled` or `disabled`.
    alert_status: Option<String>,
}

/// Reads the branches of a project and its open merge requests that changed after the cursor,
/// or all of them with no cursor. `started` is the start of the poll.
pub(super) async fn list(
    ctx: &IntegrationContext,
    token: &Hidden,
    project: &str,
    updated_after: Option<OffsetDateTime>,
    started: OffsetDateTime,
) -> Result<Listed, ProviderError> {
    // Read the branches.
    let path = format!("/projects/{project}/repository/branches");
    let branches: Vec<Branch> = get(ctx, token, &path).list(None).await?;

    // Read the open merge requests that changed.
    let path = format!("/projects/{project}/merge_requests");
    let mut call = get(ctx, token, &path).query("state", "opened");
    if let Some(after) = updated_after.and_then(rfc3339) {
        call = call.query("updated_after", after);
    }
    let requests: Vec<MergeRequest> = call.list(None).await?;
    let mut pulls = Vec::new();
    for request in requests {
        pulls.push(merge_request(ctx, token, request).await);
    }

    // Make the heads, and the cursors of the next poll.
    let branch = |branch: Branch| Head {
        kind: HeadKind::Branch(branch.name),
        commit: EventCommit {
            sha: branch.commit.id,
            message: branch.commit.message,
            author_login: None,
            avatar: None,
        },
    };
    Ok(Listed {
        branches: Some(branches.into_iter().map(branch).collect()),
        branches_cursor: Cursor::default(),
        pulls: Some(pulls.into_iter().flatten().collect()),
        all_pulls: updated_after.is_none(),
        pulls_cursor: Cursor {
            etag: None,
            updated_after: Some(started - OVERLAP),
        },
    })
}

/// Returns `true` while GitLab calls the webhook of a project: not disabled, and not deleted.
pub(super) async fn hook_works(
    ctx: &IntegrationContext,
    token: &Hidden,
    project: &str,
    hook: &str,
) -> Result<bool, ProviderError> {
    let path = format!("/projects/{project}/hooks/{hook}");
    match get(ctx, token, &path).read::<Hook>().await {
        Ok(found) => Ok(found
            .alert_status
            .as_deref()
            .is_none_or(|state| state == EXECUTABLE)),
        Err(ProviderError::NotFound) => Ok(false),
        Err(error) => Err(error),
    }
}

/// Starts a read of a path with the token, by pages of 100.
fn get<'a>(ctx: &'a IntegrationContext, token: &'a Hidden, path: &str) -> Call<'a> {
    ctx.clients
        .gitlab
        .get(path)
        .auth(Auth::PrivateToken(token))
        .query("per_page", PAGE)
}

/// Returns the head of an open merge request, or `None` for one with no commit. The source
/// project of a fork is named when GitLab shows it to the service account, else it is its id.
async fn merge_request(
    ctx: &IntegrationContext,
    token: &Hidden,
    request: MergeRequest,
) -> Option<Head> {
    // Find the source project of a fork.
    let sha = request.sha?;
    let fork = match request.source_project_id == request.target_project_id {
        true => None,
        false => {
            let source = request.source_project_id.to_string();
            let path = format!("/projects/{source}");
            let found = get(ctx, token, &path).read::<Source>().await;
            Some(found.map_or(source, |found| found.path_with_namespace))
        }
    };

    // Make the head.
    Some(Head {
        kind: HeadKind::Pull {
            pull: PullRequestRef {
                number: request.iid,
                title: request.title,
                url: request.web_url,
                source_branch: request.source_branch,
                target_branch: request.target_branch,
            },
            draft: request.draft,
            fork,
        },
        commit: EventCommit {
            sha,
            message: None,
            author_login: request.author.as_ref().map(|user| user.username.clone()),
            avatar: request.author.and_then(|user| user.avatar_url),
        },
    })
}

/// Writes a time in RFC 3339, in UTC, to the second.
fn rfc3339(at: OffsetDateTime) -> Option<String> {
    let at = chrono::DateTime::from_timestamp(at.unix_timestamp(), 0)?;
    Some(at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}
