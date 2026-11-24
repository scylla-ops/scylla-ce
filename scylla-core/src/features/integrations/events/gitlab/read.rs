//! The Git events of the GitLab webhooks: a push to a branch, a new tag, and an action on a
//! merge request.

use scylla_domain::integrations::events::{
    EventCommit, EventRepository, GitChange, GitEvent, NO_COMMIT,
};
use scylla_domain::integrations::sources::PullRequestRef;
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::git_rules::PullRequestAction;

use super::payload::{MergeRequest, Payload, Project};
use crate::features::integrations::events::dispatch::Incoming;

/// What a GitLab delivery tells.
pub(crate) struct Read {
    /// The id of the GitLab project of the delivery.
    pub(crate) project: Option<String>,
    /// The kind of the event, such as `push`.
    pub(crate) event: String,
    /// The action on a merge request, such as `open`.
    pub(crate) action: Option<String>,
    /// What the delivery asks for.
    pub(crate) incoming: Incoming,
}

/// Reads a delivery with the id `key`. A body that is not the JSON of an event is refused.
pub(crate) fn read(key: &str, body: &[u8]) -> Result<Read, String> {
    // Read the parts that each delivery has.
    let payload: Payload = serde_json::from_slice(body)
        .map_err(|_| "the body is not the JSON of an event".to_owned())?;
    let project = payload.project.as_ref().map(|found| found.id.to_string());
    let action = payload
        .object_attributes
        .as_ref()
        .and_then(|request| request.action.clone());
    let event = payload.object_kind.clone();

    // Read the event.
    let incoming = match (payload.object_kind.as_str(), payload.project.as_ref()) {
        ("push" | "tag_push", Some(found)) => push(key, found, &payload),
        ("merge_request", Some(found)) => match &payload.object_attributes {
            Some(request) => merge_request(key, found, request, &payload),
            None => Incoming::Ignored,
        },
        _ => Incoming::Ignored,
    };
    Ok(Read {
        project,
        event,
        action,
        incoming,
    })
}

/// Returns the Git event of a push to a branch or of a new tag. A deleted ref gives nothing.
fn push(key: &str, project: &Project, payload: &Payload) -> Incoming {
    // Find the new head, and refuse a deleted ref.
    let sha = payload
        .checkout_sha
        .clone()
        .or_else(|| payload.after.clone())
        .unwrap_or_default();
    let deleted = payload.after.as_deref() == Some(NO_COMMIT);
    if deleted || sha.is_empty() || sha == NO_COMMIT {
        return Incoming::Ignored;
    }

    // Read the branch or the tag.
    let git_ref = payload.git_ref.as_deref().unwrap_or_default();
    let change = if let Some(branch) = git_ref.strip_prefix("refs/heads/") {
        GitChange::Push {
            branch: branch.to_owned(),
        }
    } else if let Some(name) = git_ref.strip_prefix("refs/tags/") {
        GitChange::Tag {
            name: name.to_owned(),
        }
    } else {
        return Incoming::Ignored;
    };

    // Take the message of the head commit.
    let message = payload
        .commits
        .iter()
        .find(|commit| commit.id == sha)
        .map(|commit| commit.message.clone());
    Incoming::Git(Box::new(GitEvent {
        provider: VcsProvider::GitLab,
        key: key.to_owned(),
        repository: repository_of(project),
        change,
        commit: EventCommit {
            sha,
            message,
            author_login: payload.user_username.clone(),
            avatar: payload.user_avatar.clone(),
        },
        polled: false,
    }))
}

/// Returns the Git event of an action on a merge request: open, an update with new commits or
/// out of draft, reopen, close and merge. Another action gives nothing.
fn merge_request(
    key: &str,
    project: &Project,
    request: &MergeRequest,
    payload: &Payload,
) -> Incoming {
    // Find what happened.
    let ready = payload
        .changes
        .as_ref()
        .and_then(|changes| changes.get("draft"))
        .is_some_and(|draft| draft["previous"] == true && draft["current"] == false);
    let action = match (request.action.as_deref(), &request.oldrev, ready) {
        (Some("open"), _, _) => Some(PullRequestAction::Opened),
        (Some("reopen"), _, _) => Some(PullRequestAction::Reopened),
        (Some("update"), Some(_), _) => Some(PullRequestAction::NewCommits),
        (Some("update"), None, true) => Some(PullRequestAction::ReadyForReview),
        (Some("close" | "merge"), _, _) => None,
        _ => return Incoming::Ignored,
    };
    let Some(head) = &request.last_commit else {
        return Incoming::Ignored;
    };

    // Make the change: a fork has another source project.
    let fork = (request.source_project_id != request.target_project_id).then(|| {
        request.source.as_ref().map_or_else(
            || request.source_project_id.to_string(),
            |source| source.path_with_namespace.clone(),
        )
    });
    let change = match action {
        None => GitChange::Closed {
            number: request.iid,
        },
        Some(action) => GitChange::PullRequest {
            pull: PullRequestRef {
                number: request.iid,
                title: request.title.clone(),
                url: request.url.clone(),
                source_branch: request.source_branch.clone(),
                target_branch: request.target_branch.clone(),
            },
            action,
            draft: request.draft,
            fork_repository: fork,
        },
    };
    Incoming::Git(Box::new(GitEvent {
        provider: VcsProvider::GitLab,
        key: key.to_owned(),
        repository: repository_of(project),
        change,
        commit: EventCommit {
            sha: head.id.clone(),
            message: Some(head.message.clone()),
            author_login: payload.user.as_ref().map(|user| user.username.clone()),
            avatar: payload
                .user
                .as_ref()
                .and_then(|user| user.avatar_url.clone()),
        },
        polled: false,
    }))
}

/// Returns the repository of an event.
fn repository_of(project: &Project) -> EventRepository {
    EventRepository {
        external_id: project.id.to_string(),
        full_name: project.path_with_namespace.clone(),
        web_url: project.web_url.clone(),
    }
}
