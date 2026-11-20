//! The webhooks of GitHub: `push`, `pull_request`, `check_run`, `check_suite`, `installation` and
//! `installation_repositories`. Each one becomes what the dispatch runs.

use std::collections::BTreeMap;

use scylla_domain::integrations::events::{
    EventCommit, EventRepository, GitChange, GitEvent, NO_COMMIT,
};
use scylla_domain::integrations::sources::PullRequestRef;
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::git_rules::PullRequestAction;
use serde::de::DeserializeOwned;

use super::payload::{CheckEvent, Envelope, Listed, PullEvent, Push, Repositories, Repository};
use crate::features::integrations::events::dispatch::Incoming;
use crate::features::integrations::events::installations::InstallationChange;
use crate::features::integrations::events::rerun::RerunRequest;

/// What a GitHub delivery tells.
pub(crate) struct Read {
    /// The id of the installation of the delivery.
    pub(crate) installation: Option<String>,
    /// The id of the repository of the delivery.
    pub(crate) repository: Option<String>,
    /// The action, such as `opened`.
    pub(crate) action: Option<String>,
    /// What the delivery asks for.
    pub(crate) incoming: Incoming,
}

/// Reads a delivery of the event `event` with the id `key`. A body that is not the JSON of the
/// event is refused.
pub(crate) fn read(event: &str, key: &str, body: &[u8]) -> Result<Read, String> {
    // Read the parts that each delivery has.
    let envelope: Envelope = parse(body)?;
    let installation = envelope.installation.as_ref().map(|found| found.id);
    let action = envelope.action.clone();
    let repository = envelope
        .repository
        .as_ref()
        .map(|found| found.id.to_string());

    // Read the event.
    let incoming = match (event, action.as_deref(), installation) {
        ("push", _, _) => push(key, &envelope, parse(body)?),
        ("pull_request", Some(action), _) => pull_request(key, &envelope, action, parse(body)?),
        ("check_run" | "check_suite", Some("rerequested"), _) => {
            let check: CheckEvent = parse(body)?;
            match check.check_run.or(check.check_suite) {
                Some(check) => Incoming::Rerun(RerunRequest {
                    sha: check.head_sha,
                    check: check.external_id.filter(|id| !id.is_empty()),
                    login: envelope.sender.as_ref().map(|user| user.login.clone()),
                }),
                None => Incoming::Ignored,
            }
        }
        ("installation", Some(action), Some(id)) => {
            let permissions = envelope.installation.map(|found| found.permissions);
            installation_change(action, id.to_string(), permissions.unwrap_or_default())
        }
        ("installation_repositories", _, Some(id)) => {
            let listed: Repositories = parse(body)?;
            let ids = |list: Vec<Listed>| list.into_iter().map(|item| item.id.to_string());
            Incoming::Installation(InstallationChange::Repositories {
                installation: id.to_string(),
                all: listed.repository_selection == "all",
                added: ids(listed.repositories_added).collect(),
                removed: ids(listed.repositories_removed).collect(),
            })
        }
        _ => Incoming::Ignored,
    };
    Ok(Read {
        installation: installation.map(|id| id.to_string()),
        repository,
        action,
        incoming,
    })
}

/// Returns the Git event of a push to a branch or of a new tag. A deleted ref gives nothing.
fn push(key: &str, envelope: &Envelope, push: Push) -> Incoming {
    // Refuse a delivery with no repository, and a deleted ref.
    let Some(repository) = &envelope.repository else {
        return Incoming::Ignored;
    };
    if push.deleted || push.after == NO_COMMIT {
        return Incoming::Ignored;
    }

    // Read the branch or the tag.
    let change = if let Some(branch) = push.git_ref.strip_prefix("refs/heads/") {
        GitChange::Push {
            branch: branch.to_owned(),
        }
    } else if let Some(name) = push.git_ref.strip_prefix("refs/tags/") {
        GitChange::Tag {
            name: name.to_owned(),
        }
    } else {
        return Incoming::Ignored;
    };

    // Take the author of the head commit.
    let sender = envelope.sender.as_ref();
    let head = push.head_commit;
    let login = head
        .as_ref()
        .and_then(|head| head.author.as_ref())
        .and_then(|author| author.username.clone())
        .or_else(|| sender.map(|user| user.login.clone()));
    let avatar = sender
        .filter(|user| Some(&user.login) == login.as_ref())
        .and_then(|user| user.avatar_url.clone());
    Incoming::Git(Box::new(GitEvent {
        provider: VcsProvider::GitHub,
        key: key.to_owned(),
        repository: repository_of(repository),
        change,
        commit: EventCommit {
            sha: push.after,
            message: head.map(|head| head.message),
            author_login: login,
            avatar,
        },
        polled: false,
    }))
}

/// Returns the Git event of an action on a pull request. An action that starts no run gives
/// nothing.
fn pull_request(key: &str, envelope: &Envelope, action: &str, event: PullEvent) -> Incoming {
    // Read the action, and refuse an action that starts no run.
    let Some(repository) = &envelope.repository else {
        return Incoming::Ignored;
    };
    let pull = event.pull_request;
    let action = match action {
        "opened" => Some(PullRequestAction::Opened),
        "synchronize" => Some(PullRequestAction::NewCommits),
        "reopened" => Some(PullRequestAction::Reopened),
        "ready_for_review" => Some(PullRequestAction::ReadyForReview),
        "closed" => None,
        _ => return Incoming::Ignored,
    };

    // Find the fork of the source branch, and make the change.
    let fork = match (&pull.head.repo, &pull.base.repo) {
        (Some(head), Some(base)) if head.id == base.id => None,
        (Some(head), _) => Some(head.full_name.clone()),
        (None, _) => Some(
            pull.head
                .label
                .split(':')
                .next()
                .unwrap_or_default()
                .to_owned(),
        ),
    };
    let change = match action {
        None => GitChange::Closed {
            number: pull.number,
        },
        Some(action) => GitChange::PullRequest {
            pull: PullRequestRef {
                number: pull.number,
                title: pull.title,
                url: pull.html_url,
                source_branch: pull.head.name,
                target_branch: pull.base.name,
            },
            action,
            draft: pull.draft,
            fork_repository: fork,
        },
    };

    // Make the event, with the head of the source branch.
    Incoming::Git(Box::new(GitEvent {
        provider: VcsProvider::GitHub,
        key: key.to_owned(),
        repository: repository_of(repository),
        change,
        commit: EventCommit {
            sha: pull.head.sha,
            message: None,
            author_login: pull.user.as_ref().map(|user| user.login.clone()),
            avatar: pull.user.and_then(|user| user.avatar_url),
        },
        polled: false,
    }))
}

/// Returns the change of an installation for an action of `installation`.
fn installation_change(
    action: &str,
    installation: String,
    permissions: BTreeMap<String, String>,
) -> Incoming {
    let change = match action {
        "deleted" => InstallationChange::Deleted { installation },
        "suspend" => InstallationChange::Suspended { installation },
        "unsuspend" => InstallationChange::Unsuspended { installation },
        "new_permissions_accepted" => InstallationChange::Permissions {
            installation,
            permissions,
        },
        _ => return Incoming::Ignored,
    };
    Incoming::Installation(change)
}

/// Returns the repository of an event.
fn repository_of(repository: &Repository) -> EventRepository {
    EventRepository {
        external_id: repository.id.to_string(),
        full_name: repository.full_name.clone(),
        web_url: repository.html_url.clone(),
    }
}

/// Reads a JSON body. The error has no part of the body.
fn parse<T: DeserializeOwned>(body: &[u8]) -> Result<T, String> {
    serde_json::from_slice(body).map_err(|_| "the body is not the JSON of the event".to_owned())
}
