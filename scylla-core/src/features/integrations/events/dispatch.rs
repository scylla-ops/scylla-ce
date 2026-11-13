//! The dispatch of what a webhook or a poll brings: a Git event goes to the runs of each linked
//! project of its repository, a re-run goes to the re-runs, and a change of an installation goes
//! to its connections.

use scylla_domain::integrations::apps::VcsAppId;
use scylla_domain::integrations::connections::ConnectionState;
use scylla_domain::integrations::events::{GitChange, GitEvent};
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::ProjectId;

use super::installations::{self, InstallationChange};
use super::rerun::{self, RerunRequest};
use super::runs;
use super::store::{self, Delivery};
use crate::features::integrations::links::store::{self as links, Linked};
use crate::features::integrations::polling::heads;
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// What a delivery asks for.
pub(crate) enum Incoming {
    /// A Git event of a repository.
    Git(Box<GitEvent>),
    /// A person asked GitHub for a new run of a check.
    Rerun(RerunRequest),
    /// A change of an installation of the GitHub App.
    Installation(InstallationChange),
    /// A delivery that Scylla does not use.
    Ignored,
}

/// The linked projects that an event goes to.
#[derive(Clone, Debug)]
pub(crate) enum Scope {
    /// Each link of a repository of a GitHub installation.
    GitHub {
        /// The App of the instance, or `None` for the App of the settings.
        app: Option<VcsAppId>,
        /// The id of the installation.
        installation: String,
        /// The id of the repository, or `None` for a delivery with no repository.
        repository: Option<String>,
    },
    /// Each GitLab link of a project of GitLab.
    GitLab {
        /// The id of the GitLab project.
        repository: String,
    },
    /// The link of one Scylla project: the project of a GitLab webhook.
    Project(ProjectId),
}

/// Runs what a delivery asks for. A failure of one project goes to the log, and the other
/// projects still get the event. Returns the first failure.
pub(crate) async fn run(
    ctx: &IntegrationContext,
    scope: &Scope,
    incoming: &Incoming,
) -> Result<(), Error> {
    match incoming {
        Incoming::Git(event) => git(ctx, scope, event).await,
        Incoming::Rerun(request) => {
            let mut first = Ok(());
            for linked in active_links(ctx, scope).await? {
                let done = rerun::handle(ctx, &linked, request).await;
                first = first.and(logged(&linked, done));
            }
            first
        }
        Incoming::Installation(change) => match scope {
            Scope::GitHub { app, .. } => installations::apply(ctx, *app, change).await,
            _ => Ok(()),
        },
        Incoming::Ignored => Ok(()),
    }
}

/// Gives a Git event to each active link of its scope: it keeps the head, so a poll does not
/// find it again, then it starts the runs.
async fn git(ctx: &IntegrationContext, scope: &Scope, event: &GitEvent) -> Result<(), Error> {
    let mut first = Ok(());
    for linked in active_links(ctx, scope).await? {
        // Keep the head of the ref, or forget the head of a closed pull request.
        let project = linked.link.project_id;
        let mut tx = ctx.pool.begin().await?;
        match &event.change {
            GitChange::Push { .. } | GitChange::PullRequest { .. } => {
                heads::keep(&mut tx, project, &event.head_ref(), &event.commit.sha).await?;
            }
            GitChange::Closed { .. } => heads::forget(&mut tx, project, &event.head_ref()).await?,
            GitChange::Tag { .. } => {}
        }
        tx.commit().await?;

        // Start the runs of the link.
        let done = runs::handle(ctx, &linked, event).await;
        first = first.and(logged(&linked, done));
    }
    first
}

/// Returns the links of a scope whose link and connection are active.
async fn active_links(ctx: &IntegrationContext, scope: &Scope) -> Result<Vec<Linked>, Error> {
    // Find the projects of the scope.
    let mut tx = ctx.pool.begin().await?;
    let projects = match scope {
        Scope::GitHub {
            app,
            installation,
            repository: Some(repository),
        } => store::github_projects(&mut tx, *app, installation, repository).await?,
        Scope::GitHub { .. } => Vec::new(),
        Scope::GitLab { repository } => {
            store::projects_of(&mut tx, VcsProvider::GitLab, repository).await?
        }
        Scope::Project(project) => vec![*project],
    };

    // Read their links, and keep the active ones.
    let mut found = Vec::new();
    for project in projects {
        if let Some(linked) = links::find(&mut tx, project).await? {
            if linked.link.state == ConnectionState::Active {
                found.push(linked);
            }
        }
    }
    tx.commit().await?;
    Ok(found)
}

/// Logs the failure of the work of one project, and returns it.
fn logged(linked: &Linked, done: Result<(), Error>) -> Result<(), Error> {
    if let Err(error) = &done {
        let project = linked.link.project_id;
        tracing::warn!(%project, %error, "an event of the provider did not run");
    }
    done
}

/// Runs the work of a new delivery, then keeps its end and the error of a failure.
pub(crate) async fn finish(
    ctx: &IntegrationContext,
    provider: VcsProvider,
    key: &str,
    scope: Option<&Scope>,
    incoming: &Incoming,
) {
    let done = match scope {
        Some(scope) => run(ctx, scope, incoming).await,
        None => Ok(()),
    };
    let error = done.err().map(|error| error.to_string());
    let kept = async {
        let mut tx = ctx.pool.begin().await?;
        store::processed(&mut tx, provider, key, error.as_deref()).await?;
        tx.commit().await?;
        Ok::<(), Error>(())
    };
    if let Err(error) = kept.await {
        tracing::warn!(%error, key, "the end of a delivery was not kept");
    }
}

/// Gives a Git event that a poll found to the path of a webhook: its key is kept one time, then
/// the event runs. Returns `false` when the key was kept before.
pub(crate) async fn polled(
    ctx: &IntegrationContext,
    delivery: &Delivery<'_>,
    scope: &Scope,
    event: GitEvent,
) -> Result<bool, Error> {
    let mut tx = ctx.pool.begin().await?;
    let new = store::receive(&mut tx, delivery).await?;
    tx.commit().await?;
    if new {
        let incoming = Incoming::Git(Box::new(event));
        finish(ctx, delivery.provider, delivery.key, Some(scope), &incoming).await;
    }
    Ok(new)
}
