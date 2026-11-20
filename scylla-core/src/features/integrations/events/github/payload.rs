//! The payloads of the GitHub webhooks that Scylla reads. Each struct has only the fields that
//! Scylla uses.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The parts that each delivery has.
#[derive(Deserialize)]
pub(super) struct Envelope {
    /// The action.
    pub(super) action: Option<String>,
    /// The installation.
    pub(super) installation: Option<Installation>,
    /// The repository.
    pub(super) repository: Option<Repository>,
    /// The person that made the event.
    pub(super) sender: Option<User>,
}

/// An installation of the App.
#[derive(Deserialize)]
pub(super) struct Installation {
    /// Its id.
    pub(super) id: u64,
    /// Its permissions, by name.
    #[serde(default)]
    pub(super) permissions: BTreeMap<String, String>,
}

/// A repository.
#[derive(Clone, Deserialize)]
pub(super) struct Repository {
    /// Its id.
    pub(super) id: u64,
    /// Its full name.
    pub(super) full_name: String,
    /// Its page.
    #[serde(default)]
    pub(super) html_url: String,
}

/// An account of GitHub.
#[derive(Clone, Deserialize)]
pub(super) struct User {
    /// The login.
    pub(super) login: String,
    /// The address of the avatar.
    pub(super) avatar_url: Option<String>,
}

/// The payload of `push`.
#[derive(Deserialize)]
pub(super) struct Push {
    /// The full ref.
    #[serde(rename = "ref")]
    pub(super) git_ref: String,
    /// The new head.
    pub(super) after: String,
    /// `true` when the push deletes the ref.
    #[serde(default)]
    pub(super) deleted: bool,
    /// The head commit.
    pub(super) head_commit: Option<HeadCommit>,
}

/// The head commit of a push.
#[derive(Deserialize)]
pub(super) struct HeadCommit {
    /// The message.
    pub(super) message: String,
    /// The author.
    pub(super) author: Option<CommitAuthor>,
}

/// The author of a commit.
#[derive(Deserialize)]
pub(super) struct CommitAuthor {
    /// The GitHub login of the author, if GitHub knows one.
    pub(super) username: Option<String>,
}

/// The payload of `pull_request`.
#[derive(Deserialize)]
pub(super) struct PullEvent {
    /// The pull request.
    pub(super) pull_request: Pull,
}

/// A pull request.
#[derive(Deserialize)]
pub(super) struct Pull {
    /// The number.
    pub(super) number: u32,
    /// The title.
    pub(super) title: String,
    /// The page.
    pub(super) html_url: String,
    /// `true` for a draft.
    #[serde(default)]
    pub(super) draft: bool,
    /// The source branch.
    pub(super) head: Side,
    /// The target branch.
    pub(super) base: Side,
    /// The author.
    pub(super) user: Option<User>,
}

/// A side of a pull request.
#[derive(Deserialize)]
pub(super) struct Side {
    /// The branch.
    #[serde(rename = "ref")]
    pub(super) name: String,
    /// The commit.
    pub(super) sha: String,
    /// The repository, or `None` when the fork was deleted.
    pub(super) repo: Option<Repository>,
    /// `owner:branch`.
    #[serde(default)]
    pub(super) label: String,
}

/// The payload of `check_run` and of `check_suite`.
#[derive(Deserialize)]
pub(super) struct CheckEvent {
    /// The check run.
    pub(super) check_run: Option<Check>,
    /// The check suite.
    pub(super) check_suite: Option<Check>,
}

/// A check run or a check suite.
#[derive(Deserialize)]
pub(super) struct Check {
    /// The commit.
    pub(super) head_sha: String,
    /// The id that Scylla gave to a check run.
    pub(super) external_id: Option<String>,
}

/// The payload of `installation_repositories`.
#[derive(Deserialize)]
pub(super) struct Repositories {
    /// `all` or `selected`.
    #[serde(default)]
    pub(super) repository_selection: String,
    /// The repositories that the installation now sees.
    #[serde(default)]
    pub(super) repositories_added: Vec<Listed>,
    /// The repositories that the installation no longer sees.
    #[serde(default)]
    pub(super) repositories_removed: Vec<Listed>,
}

/// A repository in a list of an installation.
#[derive(Deserialize)]
pub(super) struct Listed {
    /// Its id.
    pub(super) id: u64,
}
