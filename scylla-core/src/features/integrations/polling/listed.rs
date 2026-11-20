//! What a poll reads on the provider: the heads of the branches and of the open pull requests,
//! and the cursor of each list for the next poll.

use std::collections::HashMap;

use scylla_domain::integrations::events::{pull_ref, EventCommit};
use scylla_domain::integrations::sources::PullRequestRef;
use scylla_domain::integrations::VcsProvider;
use time::OffsetDateTime;

use super::store::heads::Row;

/// The name of the cursor of the list of the branches.
pub(super) const BRANCHES: &str = "branches";

/// The name of the cursor of the list of the open pull requests.
pub(super) const PULLS: &str = "pulls";

/// The heads that a poll read.
#[derive(Debug, Default)]
pub(super) struct Listed {
    /// The heads of all the branches, or `None` when the list did not change.
    pub(super) branches: Option<Vec<Head>>,
    /// The cursor of the list of the branches.
    pub(super) branches_cursor: Cursor,
    /// The heads of the open pull requests, or `None` when the list did not change.
    pub(super) pulls: Option<Vec<Head>>,
    /// `true` when `pulls` has all the open pull requests, `false` when it has only the ones
    /// that changed since the cursor.
    pub(super) all_pulls: bool,
    /// The cursor of the list of the open pull requests.
    pub(super) pulls_cursor: Cursor,
}

/// The cursor of a list, for the next poll.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Cursor {
    /// The ETag of the list.
    pub(super) etag: Option<String>,
    /// The time from which the next read asks for the changes.
    pub(super) updated_after: Option<OffsetDateTime>,
}

/// The head of a branch or of an open pull request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Head {
    /// The branch or the pull request.
    pub(super) kind: HeadKind,
    /// The head commit.
    pub(super) commit: EventCommit,
}

/// What a head is the head of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum HeadKind {
    /// A branch, with its name.
    Branch(String),
    /// An open pull request.
    Pull {
        /// The pull request.
        pull: PullRequestRef,
        /// `true` for a draft.
        draft: bool,
        /// The full name of the fork of its source branch, or `None`.
        fork: Option<String>,
    },
}

impl Head {
    /// Returns the full Git ref of the head on a provider.
    pub(super) fn git_ref(&self, provider: VcsProvider) -> String {
        match &self.kind {
            HeadKind::Branch(name) => format!("refs/heads/{name}"),
            HeadKind::Pull { pull, .. } => pull_ref(provider, pull.number),
        }
    }

    /// Returns a commit with only a SHA.
    pub(super) fn commit_of(sha: String) -> EventCommit {
        EventCommit {
            sha,
            message: None,
            author_login: None,
            avatar: None,
        }
    }
}

/// What the last poll of a link saw: the head of each ref, and the cursor of each list.
#[derive(Debug, Default)]
pub(super) struct Seen {
    /// The last head of each ref.
    pub(super) heads: HashMap<String, String>,
    /// The cursor of each list that a poll read.
    pub(super) cursors: HashMap<String, Cursor>,
}

impl Seen {
    /// Reads the rows of the poll state of a link.
    pub(super) fn of(rows: Vec<Row>) -> Self {
        let mut seen = Self::default();
        for row in rows {
            match row.sha {
                Some(sha) if row.name.starts_with("refs/") => {
                    seen.heads.insert(row.name, sha);
                }
                _ => {
                    let cursor = Cursor {
                        etag: row.etag,
                        updated_after: row.updated_after,
                    };
                    seen.cursors.insert(row.name, cursor);
                }
            }
        }
        seen
    }

    /// Returns `true` when a poll read the list before: its heads are known, so a new head is an
    /// event.
    pub(super) fn knows(&self, list: &str) -> bool {
        self.cursors.contains_key(list)
    }

    /// Returns the cursor of a list.
    pub(super) fn cursor(&self, list: &str) -> Cursor {
        self.cursors.get(list).cloned().unwrap_or_default()
    }

    /// Returns `true` when a ref has another head than the last one seen, or no head seen.
    pub(super) fn is_new(&self, git_ref: &str, sha: &str) -> bool {
        self.heads.get(git_ref).is_none_or(|seen| seen != sha)
    }
}
