//! The checkout of a job at its dispatch: the repository of its project, its commit, the ref of
//! a pull request from a fork, and a short-lived token that can only read the repository.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use scylla_domain::integrations::connections::ConnectionState;
use scylla_domain::integrations::sources::JobSource;
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::jobs::Job;
use scylla_domain::projects::pipeline_files::CheckoutSettings;
use scylla_extension::Rejection;

use super::{github, gitlab, store};
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::Hidden;
use crate::Error;

/// The checkout that an agent gets with a job.
pub(crate) struct Checkout {
    /// The address of the repository with no credentials.
    pub(crate) clone_url: String,
    /// The commit.
    pub(crate) sha: String,
    /// The ref to fetch for a commit of a fork, or an empty text for the commit.
    pub(crate) fetch_ref: String,
    /// The header with the short-lived token.
    pub(crate) auth_header: Hidden,
    /// The depth, the submodules and the LFS files.
    pub(crate) settings: CheckoutSettings,
}

/// Returns the checkout of a job, or `None` for a job with no checkout. Returns the reason of a
/// failure, with no token: the job fails with it.
pub(crate) async fn prepare(
    ctx: &IntegrationContext,
    job: &Job,
) -> Result<Option<Checkout>, String> {
    let (Some(run), Some(source)) = (&job.checkout, &job.source) else {
        return Ok(None);
    };
    let checkout = make(ctx, job, source, &run.settings).await;
    checkout
        .map(Some)
        .map_err(|error| format!("the checkout cannot start: {error}"))
}

/// Makes the checkout of a job with a source: the link of its project must be active and must
/// still be the repository of the source.
async fn make(
    ctx: &IntegrationContext,
    job: &Job,
    source: &JobSource,
    settings: &CheckoutSettings,
) -> Result<Checkout, Error> {
    // Read the repository of the project, and refuse a link that cannot serve.
    let mut tx = ctx.pool.begin().await?;
    let target = store::target(&mut tx, job.project_id).await?;
    tx.commit().await?;
    let message = "the project has no linked repository";
    let target = target.ok_or_else(|| refused("NO_REPOSITORY_LINK", message))?;
    if target.state != ConnectionState::Active {
        let message = format!("the repository of the project is {}", target.state.as_str());
        return Err(refused("REPOSITORY_NOT_ACTIVE", message));
    }
    if !target.full_name.eq_ignore_ascii_case(&source.repository) {
        let message = format!("the project is now linked to {}", target.full_name);
        return Err(refused("REPOSITORY_CHANGED", message));
    }

    // Get a token that can only read the repository.
    let auth_header = match target.provider {
        VcsProvider::GitHub => github::header(ctx, &target).await?,
        VcsProvider::GitLab => gitlab::header(ctx, &target, job).await?,
    };
    Ok(Checkout {
        clone_url: format!("{}.git", target.web_url.trim_end_matches('/')),
        sha: source.sha.clone(),
        fetch_ref: fetch_ref(source),
        auth_header,
        settings: settings.clone(),
    })
}

/// Returns the ref that holds the commit of a pull request from a fork, in the base repository:
/// `refs/pull/<n>/head` on GitHub, `refs/merge-requests/<iid>/head` on GitLab. Another commit
/// is in the repository, so it gives an empty text.
pub(super) fn fetch_ref(source: &JobSource) -> String {
    let pull = source.pull_requests.first().filter(|_| source.from_fork());
    match (pull, source.provider) {
        (Some(pull), VcsProvider::GitHub) => format!("refs/pull/{}/head", pull.number),
        (Some(pull), VcsProvider::GitLab) => format!("refs/merge-requests/{}/head", pull.number),
        (None, _) => String::new(),
    }
}

/// Returns the HTTP header of a basic authentication.
pub(super) fn basic(user: &str, token: &Hidden) -> Hidden {
    let credential = STANDARD.encode(format!("{user}:{}", token.expose()));
    Hidden::new(format!("Authorization: Basic {credential}"))
}

/// Returns a refusal with a stable code.
fn refused(code: &'static str, message: impl Into<String>) -> Error {
    Error::Rejected(Rejection::new(code, message))
}
