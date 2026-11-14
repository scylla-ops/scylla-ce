//! The commit of a manual run: a branch, a tag or a SHA of the linked repository, as the provider
//! gives it.

use scylla_domain::integrations::connections::ConnectionState;
use scylla_domain::integrations::sources::{title_of, GitRef, JobSource, SourceEvent};
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::ProjectId;
use scylla_extension::Rejection;

use super::{github, gitlab, store};
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::ProviderError;
use crate::Error;

/// A commit, as a provider gives it.
pub(super) struct Commit {
    /// The full SHA.
    pub(super) sha: String,
    /// The message.
    pub(super) message: String,
    /// The login of the author on the provider, if the provider knows it.
    pub(super) author_login: Option<String>,
    /// The avatar of the author.
    pub(super) avatar: Option<String>,
}

/// Returns the source of a manual run of a project on a branch, a tag or a SHA of its linked
/// repository. A project with no link gives `NO_REPOSITORY_LINK`, a link that is not active gives
/// `REPOSITORY_NOT_ACTIVE`, and a ref that the provider does not know gives `NOT_FOUND`.
pub(crate) async fn resolve(
    ctx: &IntegrationContext,
    project: ProjectId,
    git_ref: &GitRef,
) -> Result<JobSource, Error> {
    // Read the repository of the project, and refuse a link that cannot serve.
    let mut tx = ctx.pool.begin().await?;
    let target = store::target(&mut tx, project).await?;
    tx.commit().await?;
    let Some(target) = target else {
        let message = "the project has no linked repository";
        return Err(Error::Rejected(Rejection::new(
            "NO_REPOSITORY_LINK",
            message,
        )));
    };
    if target.state != ConnectionState::Active {
        let message = "the repository of the project cannot be used now";
        let rejection = Rejection::new("REPOSITORY_NOT_ACTIVE", message)
            .detail("state", target.state.as_str())
            .detail("reason", target.reason.as_deref().unwrap_or_default());
        return Err(Error::Rejected(rejection));
    }

    // Ask the provider for the commit.
    let commit = match target.provider {
        VcsProvider::GitHub => github::commit(ctx, &target, git_ref).await,
        VcsProvider::GitLab => gitlab::commit(ctx, &target, git_ref).await,
    };
    let commit = commit.map_err(|error| failure(project, error))?;

    // Make the source of the run.
    let (branch, tag) = match git_ref {
        GitRef::Branch(name) => (Some(name.clone()), None),
        GitRef::Tag(name) => (None, Some(name.clone())),
        GitRef::Sha(_) => (None, None),
    };
    Ok(JobSource {
        provider: target.provider,
        repository: target.full_name,
        url: target.web_url,
        sha: commit.sha,
        git_ref: git_ref.full_ref(),
        branch,
        tag,
        pull_requests: Vec::new(),
        fork_repository: None,
        author_login: commit.author_login,
        avatar: commit.avatar,
        message_title: title_of(&commit.message),
        event: SourceEvent::Manual,
        automatic: false,
        changes_pipeline_files: false,
    })
}

/// A failure of a call for a commit: an error of the server, or a refusal of the provider.
pub(super) enum Failure {
    /// The server cannot make the call.
    Server(Error),
    /// The provider refused the call, or failed.
    Provider(ProviderError),
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::Server(error)
    }
}

impl From<ProviderError> for Failure {
    fn from(error: ProviderError) -> Self {
        Self::Provider(error)
    }
}

/// Returns the error of a failed call: `NOT_FOUND` for a ref that the provider does not know,
/// else a refusal with a stable code.
fn failure(project: ProjectId, failure: Failure) -> Error {
    let error = match failure {
        Failure::Server(error) => return error,
        Failure::Provider(ProviderError::NotFound | ProviderError::Invalid(_)) => {
            return Error::NotFound("ref");
        }
        Failure::Provider(error) => error,
    };
    tracing::warn!(%project, %error, "the provider did not give the commit of a run");
    let rejection = match &error {
        ProviderError::RateLimited(until) => {
            let message = "the provider limits the calls";
            Rejection::new("PROVIDER_LIMIT", message).detail("retry_at", until.unix_timestamp())
        }
        ProviderError::Unauthorized | ProviderError::Forbidden => {
            Rejection::new("PROVIDER_REFUSED", "the provider refused the connection")
        }
        _ => Rejection::new("PROVIDER_UNAVAILABLE", "the provider is not available"),
    };
    Error::Rejected(rejection)
}
