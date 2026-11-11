//! The rules of the Git triggers: a push to some branches, a pull request into some branches, or
//! a tag. The Git triggers of the editor and the `on` table of a pipeline file both use them.

mod globs;
#[cfg(test)]
mod tests;

use globs::{any, any_or_all, check_globs};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use crate::DomainError;

/// The Git events that start a pipeline. With no rule, only a manual run starts it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GitRules {
    /// A push to a branch starts a run.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<PushRule>")]
    pub push: Option<PushRule>,
    /// A pull request (a merge request on GitLab) starts a run.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<PullRequestRule>")]
    pub pull_request: Option<PullRequestRule>,
    /// A new tag starts a run.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<TagRule>")]
    pub tag: Option<TagRule>,
}

/// The rule of a push to a branch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PushRule {
    /// The globs of the branches, for example `main` or `release/*`. No glob is each branch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branches: Vec<String>,
    /// The globs of the branches that never start a run, also when `branches` has them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    /// `true` skips a push when its head commit message has `[skip ci]` or `[ci skip]`.
    #[serde(default = "skip_ci_default", skip_serializing_if = "is_true")]
    pub skip_ci: bool,
}

/// The rule of a pull request, or of a merge request on GitLab.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PullRequestRule {
    /// The globs of the target branches. No glob is each branch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branches: Vec<String>,
    /// The actions that start a run. No action is each action.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<PullRequestAction>,
    /// `true` also runs the draft pull requests. `false` skips them.
    #[serde(default, skip_serializing_if = "is_false")]
    pub drafts: bool,
}

/// An action on a pull request that can start a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestAction {
    /// The pull request was opened.
    Opened,
    /// The source branch of the pull request has new commits.
    NewCommits,
    /// The pull request was opened again.
    Reopened,
    /// A draft pull request is ready for review.
    ReadyForReview,
}

/// The rule of a new tag.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TagRule {
    /// The globs of the tags, for example `v*`. No glob is each tag.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<String>,
}

/// A Git event, as a rule checks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerEvent<'a> {
    /// A push to a branch.
    Push {
        /// The name of the branch, with no `refs/heads/`.
        branch: &'a str,
        /// The message of the head commit.
        message: &'a str,
    },
    /// An action on a pull request.
    PullRequest {
        /// The target branch, with no `refs/heads/`.
        target: &'a str,
        /// The action.
        action: PullRequestAction,
        /// `true` for a draft pull request.
        draft: bool,
    },
    /// A new tag.
    Tag {
        /// The name of the tag, with no `refs/tags/`.
        name: &'a str,
    },
}

/// The most globs in one list.
pub const MAX_GLOBS: usize = 32;

/// The longest glob, in characters.
pub const MAX_GLOB_LEN: usize = 255;

/// Reads a rule that is set. An empty value, as `push:` with nothing in YAML, is the rule with
/// its defaults.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Some(
        Option::<T>::deserialize(deserializer)?.unwrap_or_default(),
    ))
}

/// The default of `skip_ci`: skip the commits with `[skip ci]`.
fn skip_ci_default() -> bool {
    true
}

/// Returns `true` if the flag is on, so the default of `skip_ci` is not written.
fn is_true(flag: &bool) -> bool {
    *flag
}

/// Returns `true` if the flag is off, so the default of `drafts` is not written.
fn is_false(flag: &bool) -> bool {
    !*flag
}

impl Default for PushRule {
    fn default() -> Self {
        Self {
            branches: Vec::new(),
            exclude: Vec::new(),
            skip_ci: true,
        }
    }
}

impl GitRules {
    /// Returns `true` if no rule is set: only a manual run starts the pipeline.
    pub fn is_empty(&self) -> bool {
        self.push.is_none() && self.pull_request.is_none() && self.tag.is_none()
    }

    /// Checks each list of globs, at the path `at`. Returns the path and the rule of each
    /// refusal.
    pub fn check(&self, at: &str) -> Vec<(String, DomainError)> {
        // Find the lists of globs of the rules that are set.
        let mut lists = Vec::new();
        if let Some(push) = &self.push {
            lists.push((format!("{at}.push.branches"), &push.branches));
            lists.push((format!("{at}.push.exclude"), &push.exclude));
        }
        if let Some(pull_request) = &self.pull_request {
            lists.push((
                format!("{at}.pull_request.branches"),
                &pull_request.branches,
            ));
        }
        if let Some(tag) = &self.tag {
            lists.push((format!("{at}.tag.patterns"), &tag.patterns));
        }

        // Check each list.
        lists
            .into_iter()
            .flat_map(|(path, globs)| check_globs(&path, globs))
            .collect()
    }

    /// Returns `true` if a rule starts a run for the event.
    pub fn matches(&self, event: &TriggerEvent<'_>) -> bool {
        match event {
            TriggerEvent::Push { .. } => self.push.as_ref().is_some_and(|rule| rule.matches(event)),
            TriggerEvent::PullRequest { .. } => self
                .pull_request
                .as_ref()
                .is_some_and(|rule| rule.matches(event)),
            TriggerEvent::Tag { .. } => self.tag.as_ref().is_some_and(|rule| rule.matches(event)),
        }
    }
}

impl PushRule {
    /// Returns `true` for a push to a branch of `branches` and not of `exclude`, with no skip
    /// mark when `skip_ci` is on.
    pub fn matches(&self, event: &TriggerEvent<'_>) -> bool {
        let TriggerEvent::Push { branch, message } = event else {
            return false;
        };
        if self.skip_ci && has_skip_mark(message) {
            return false;
        }
        any_or_all(&self.branches, branch) && !any(&self.exclude, branch)
    }
}

impl PullRequestRule {
    /// Returns `true` for an action of `events` on a pull request into a branch of `branches`.
    /// A draft passes only when `drafts` is on.
    pub fn matches(&self, event: &TriggerEvent<'_>) -> bool {
        // Take only a pull request, and skip a draft unless `drafts` is on.
        let TriggerEvent::PullRequest {
            target,
            action,
            draft,
        } = event
        else {
            return false;
        };
        if *draft && !self.drafts {
            return false;
        }

        // Check the action and the target branch.
        let action_passes = self.events.is_empty() || self.events.contains(action);
        action_passes && any_or_all(&self.branches, target)
    }
}

impl TagRule {
    /// Returns `true` for a tag of `patterns`.
    pub fn matches(&self, event: &TriggerEvent<'_>) -> bool {
        let TriggerEvent::Tag { name } = event else {
            return false;
        };
        any_or_all(&self.patterns, name)
    }
}

/// Returns `true` if a commit message has `[skip ci]` or `[ci skip]`, in any case.
fn has_skip_mark(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("[skip ci]") || message.contains("[ci skip]")
}
