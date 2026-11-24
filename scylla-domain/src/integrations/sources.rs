//! The commit of a job in the linked repository, the event that started the job, and the approval
//! of a job from a fork.

use super::{named, VcsProvider};
use crate::projects::pipelines::{EnvValue, EnvVar};
use crate::DomainError;

/// The longest title of a commit that a source keeps, in characters.
pub const MAX_TITLE: usize = 255;

named! {
    /// The event that started a job with a source.
    pub enum SourceEvent {
        /// A push to a branch.
        Push = "push",
        /// A pull request or a merge request: opened, new commits, reopened, or ready for review.
        PullRequest = "pull_request",
        /// A push of a tag.
        Tag = "tag",
        /// A person started the job on a branch, a tag or a commit.
        Manual = "manual",
        /// A person asked for a new run on the provider.
        ProviderRerun = "provider_rerun",
        /// A trigger of Scylla started the job at the head of the default branch: a cron or
        /// webhook trigger of a file pipeline, or a Git trigger that a person fired now.
        Trigger = "trigger",
    }
}

named! {
    /// The state of the approval of a job from a fork.
    pub enum ApprovalState {
        /// The job waits for an editor of the project.
        Waiting = "waiting",
        /// An editor approved the job. It runs with no secret value.
        Approved = "approved",
        /// An editor rejected the job. It is cancelled.
        Rejected = "rejected",
    }
}

/// A pull request (GitHub) or a merge request (GitLab) of a job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequestRef {
    /// The number: 128 for #128 on GitHub, 17 for !17 on GitLab.
    pub number: u32,
    /// The title.
    pub title: String,
    /// The page of the pull request on the provider.
    pub url: String,
    /// The source branch.
    pub source_branch: String,
    /// The target branch.
    pub target_branch: String,
}

/// The commit of a job and the event that started it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobSource {
    /// The provider.
    pub provider: VcsProvider,
    /// The full name of the repository, for example `acme/api`.
    pub repository: String,
    /// The page of the repository on the provider.
    pub url: String,
    /// The commit: 40 hexadecimal characters.
    pub sha: String,
    /// The full Git ref of the event, for example `refs/heads/main`, or `None` for a commit.
    pub git_ref: Option<String>,
    /// The branch, or `None` for a tag or a commit.
    pub branch: Option<String>,
    /// The tag, or `None` for a branch or a commit.
    pub tag: Option<String>,
    /// The pull requests of the commit. An event on the same commit adds its pull request.
    pub pull_requests: Vec<PullRequestRef>,
    /// The full name of the fork, or `None` when the commit is not from a fork.
    pub fork_repository: Option<String>,
    /// The login of the author of the commit on the provider.
    pub author_login: Option<String>,
    /// The address of the avatar of the author.
    pub avatar: Option<String>,
    /// The first line of the message of the commit.
    pub message_title: String,
    /// The event that started the job.
    pub event: SourceEvent,
    /// `true` for a job of a push, a pull request or a tag: one job for each pipeline and
    /// commit. A manual run and a re-run are not automatic.
    pub automatic: bool,
    /// `true` when the pull request changes a file in `.scylla/`.
    pub changes_pipeline_files: bool,
}

impl JobSource {
    /// Returns `true` when the commit comes from a fork. The job then gets no secret value.
    pub fn from_fork(&self) -> bool {
        self.fork_repository.is_some()
    }

    /// Returns the page of the commit on the provider.
    pub fn commit_url(&self) -> String {
        match self.provider {
            VcsProvider::GitHub => format!("{}/commit/{}", self.url, self.sha),
            VcsProvider::GitLab => format!("{}/-/commit/{}", self.url, self.sha),
        }
    }

    /// Returns the page of the changes of the first pull request on the provider, or `None` for
    /// a commit with no pull request.
    pub fn changes_url(&self) -> Option<String> {
        let first = self.pull_requests.first()?;
        Some(match self.provider {
            VcsProvider::GitHub => format!("{}/files", first.url),
            VcsProvider::GitLab => format!("{}/diffs", first.url),
        })
    }

    /// Returns the variables that each node of the job gets: the commit, the ref, the first pull
    /// request, the repository, the event and the fork. A value that the job does not have is
    /// empty.
    pub fn variables(&self) -> Vec<EnvVar> {
        let pull_request = self.pull_requests.first().map(|one| one.number.to_string());
        let values = [
            ("SCYLLA_GIT_SHA", Some(self.sha.clone())),
            ("SCYLLA_GIT_REF", self.git_ref.clone()),
            ("SCYLLA_GIT_BRANCH", self.branch.clone()),
            ("SCYLLA_GIT_TAG", self.tag.clone()),
            ("SCYLLA_PR_NUMBER", pull_request),
            ("SCYLLA_REPO", Some(self.repository.clone())),
            ("SCYLLA_REPO_URL", Some(self.url.clone())),
            ("SCYLLA_EVENT", Some(self.event.as_str().to_owned())),
            ("SCYLLA_GIT_FORK", Some(self.from_fork().to_string())),
        ];
        values
            .into_iter()
            .map(|(name, value)| EnvVar {
                name: name.to_owned(),
                value: EnvValue::Literal(value.unwrap_or_default()),
            })
            .collect()
    }
}

/// Returns the title of a commit: the first line of its message, cut at `MAX_TITLE` characters.
pub fn title_of(message: &str) -> String {
    let first = message.lines().next().unwrap_or_default().trim();
    first.chars().take(MAX_TITLE).collect()
}

/// A commit of the linked repository that a person names: a branch, a tag or a SHA.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitRef {
    /// The head of a branch, for example `main`.
    Branch(String),
    /// A tag, for example `v1.2.0`.
    Tag(String),
    /// A commit: 7 to 40 hexadecimal characters, in lower case.
    Sha(String),
}

impl GitRef {
    /// The longest name of a branch or of a tag, in characters.
    pub const MAX_NAME: usize = 255;

    /// Reads the name of a branch.
    pub fn branch(name: String) -> Result<Self, DomainError> {
        check_name(&name)?;
        Ok(Self::Branch(name))
    }

    /// Reads the name of a tag.
    pub fn tag(name: String) -> Result<Self, DomainError> {
        check_name(&name)?;
        Ok(Self::Tag(name))
    }

    /// Reads a full or a short commit SHA.
    pub fn sha(text: &str) -> Result<Self, DomainError> {
        let hex = text.chars().all(|char| char.is_ascii_hexdigit());
        if !hex || !(7..=40).contains(&text.len()) {
            return Err(DomainError::NotASha);
        }
        Ok(Self::Sha(text.to_ascii_lowercase()))
    }

    /// Returns the full Git ref, `refs/heads/<branch>` or `refs/tags/<tag>`, or `None` for a SHA.
    pub fn full_ref(&self) -> Option<String> {
        match self {
            Self::Branch(name) => Some(format!("refs/heads/{name}")),
            Self::Tag(name) => Some(format!("refs/tags/{name}")),
            Self::Sha(_) => None,
        }
    }
}

/// Checks the name of a branch or of a tag with the rules of `git check-ref-format`.
fn check_name(name: &str) -> Result<(), DomainError> {
    let bad = |char: char| char.is_whitespace() || char.is_control() || "~^:?*[\\".contains(char);
    let refused = name.is_empty()
        || name.chars().count() > GitRef::MAX_NAME
        || name.chars().any(bad)
        || name.starts_with(['-', '/', '.'])
        || name.ends_with(['/', '.'])
        || name.ends_with(".lock")
        || ["..", "//", "/.", "@{"]
            .iter()
            .any(|part| name.contains(part))
        || name == "@";
    match refused {
        true => Err(DomainError::NotARefName),
        false => Ok(()),
    }
}
