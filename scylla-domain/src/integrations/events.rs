//! A Git event of a repository, from a webhook of the provider or from a poll: a push to a
//! branch, a new tag, or a change of a pull request. The runs, the rules of the Git triggers and
//! the source of a job read it.

#[cfg(test)]
mod tests;

use super::sources::{title_of, JobSource, PullRequestRef, SourceEvent};
use super::VcsProvider;
use crate::projects::git_rules::{PullRequestAction, TriggerEvent};

/// The SHA of a ref that a push deletes.
pub const NO_COMMIT: &str = "0000000000000000000000000000000000000000";

/// The repository of an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventRepository {
    /// The id on the provider.
    pub external_id: String,
    /// The full name, for example `acme/api`.
    pub full_name: String,
    /// The page on the provider.
    pub web_url: String,
}

/// The head commit of an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventCommit {
    /// The full SHA.
    pub sha: String,
    /// The message, or `None` when the provider does not give it.
    pub message: Option<String>,
    /// The login of the author on the provider, if the provider knows it.
    pub author_login: Option<String>,
    /// The address of the avatar of the author.
    pub avatar: Option<String>,
}

/// What changed in the repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitChange {
    /// A push to a branch, with a new head.
    Push {
        /// The name of the branch, with no `refs/heads/`.
        branch: String,
    },
    /// A new tag.
    Tag {
        /// The name of the tag, with no `refs/tags/`.
        name: String,
    },
    /// A pull request (a merge request on GitLab) that can start a run.
    PullRequest {
        /// The pull request.
        pull: PullRequestRef,
        /// What happened to it.
        action: PullRequestAction,
        /// `true` for a draft.
        draft: bool,
        /// The full name of the fork of the source branch, or `None` for a branch of the
        /// repository.
        fork_repository: Option<String>,
    },
    /// A pull request that was closed or merged.
    Closed {
        /// The number of the pull request.
        number: u32,
    },
}

/// A Git event of a repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitEvent {
    /// The provider.
    pub provider: VcsProvider,
    /// The key of the event: the id of the delivery of a webhook, or `poll:<repo>:<ref>:<sha>`.
    pub key: String,
    /// The repository.
    pub repository: EventRepository,
    /// What changed.
    pub change: GitChange,
    /// The head commit.
    pub commit: EventCommit,
    /// `true` when a poll found the event, not a webhook.
    pub polled: bool,
}

impl GitEvent {
    /// Returns the key of an event that a poll found.
    pub fn poll_key(repository: &str, git_ref: &str, sha: &str) -> String {
        format!("poll:{repository}:{git_ref}:{sha}")
    }

    /// Returns the full Git ref of the head: `refs/heads/<branch>`, `refs/tags/<tag>`, or the
    /// head of a pull request (`refs/pull/<n>/head` on GitHub, `refs/merge-requests/<n>/head` on
    /// GitLab).
    pub fn head_ref(&self) -> String {
        match &self.change {
            GitChange::Push { branch } => format!("refs/heads/{branch}"),
            GitChange::Tag { name } => format!("refs/tags/{name}"),
            GitChange::PullRequest { pull, .. } => pull_ref(self.provider, pull.number),
            GitChange::Closed { number } => pull_ref(self.provider, *number),
        }
    }

    /// Returns what the rules of the Git triggers check, or `None` for a closed pull request.
    pub fn trigger(&self) -> Option<TriggerEvent<'_>> {
        match &self.change {
            GitChange::Push { branch } => Some(TriggerEvent::Push {
                branch,
                message: self.commit.message.as_deref().unwrap_or_default(),
            }),
            GitChange::Tag { name } => Some(TriggerEvent::Tag { name }),
            GitChange::PullRequest {
                pull,
                action,
                draft,
                ..
            } => Some(TriggerEvent::PullRequest {
                target: &pull.target_branch,
                action: *action,
                draft: *draft,
            }),
            GitChange::Closed { .. } => None,
        }
    }

    /// Returns the source of an automatic job of the event, or `None` for a closed pull request.
    /// The title is the first line of the message, else the title of the pull request.
    pub fn source(&self) -> Option<JobSource> {
        let (event, branch, tag, pulls, fork) = match &self.change {
            GitChange::Push { branch } => {
                (SourceEvent::Push, Some(branch.clone()), None, vec![], None)
            }
            GitChange::Tag { name } => (SourceEvent::Tag, None, Some(name.clone()), vec![], None),
            GitChange::PullRequest {
                pull,
                fork_repository,
                ..
            } => {
                let branch = fork_repository
                    .is_none()
                    .then(|| pull.source_branch.clone());
                let pulls = vec![pull.clone()];
                (
                    SourceEvent::PullRequest,
                    branch,
                    None,
                    pulls,
                    fork_repository.clone(),
                )
            }
            GitChange::Closed { .. } => return None,
        };
        let title = match (&self.commit.message, pulls.first()) {
            (Some(message), _) => title_of(message),
            (None, Some(pull)) => title_of(&pull.title),
            (None, None) => String::new(),
        };
        Some(JobSource {
            provider: self.provider,
            repository: self.repository.full_name.clone(),
            url: self.repository.web_url.clone(),
            sha: self.commit.sha.clone(),
            git_ref: Some(self.head_ref()),
            branch,
            tag,
            pull_requests: pulls,
            fork_repository: fork,
            author_login: self.commit.author_login.clone(),
            avatar: self.commit.avatar.clone(),
            message_title: title,
            event,
            automatic: true,
            changes_pipeline_files: false,
        })
    }
}

/// Returns the Git ref of the head of a pull request on a provider.
pub fn pull_ref(provider: VcsProvider, number: u32) -> String {
    match provider {
        VcsProvider::GitHub => format!("refs/pull/{number}/head"),
        VcsProvider::GitLab => format!("refs/merge-requests/{number}/head"),
    }
}
