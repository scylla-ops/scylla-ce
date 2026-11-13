//! One poll of a linked repository: read the heads of its branches and of its open pull
//! requests, keep them, and give each new head to the path of a webhook as a Git event with the
//! key `poll:<repo>:<ref>:<sha>`. A link in webhook mode moves to the polling mode when a poll
//! finds a head that no webhook told, or when GitLab disabled its webhook.

use scylla_domain::integrations::connections::{ConnectionState, DeliveryMode, VcsConnection};
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::ProjectId;
use time::{Duration, OffsetDateTime};

use super::listed::{Head, HeadKind, Listed, Seen, BRANCHES, PULLS};
use super::store::{heads, links as modes};
use super::{deliver, github, gitlab};
use crate::features::integrations::connections::credentials;
use crate::features::integrations::connections::store as connections;
use crate::features::integrations::links::store::{self as links, Linked};
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::ProviderError;
use crate::Error;

/// The time between two safety polls of a link in webhook mode, in seconds.
pub(crate) const SAFETY_SECONDS: f64 = 900.0;

/// The time with no webhook after which a head that a poll finds moves a link to polling.
const WINDOW: Duration = Duration::minutes(15);

/// The reason of a link in polling mode because the webhooks do not arrive.
pub(crate) const WEBHOOKS_NOT_ARRIVING: &str = "WEBHOOKS_NOT_ARRIVING";

/// The reason of a link in polling mode because GitLab disabled its webhook.
const WEBHOOK_DISABLED: &str = "WEBHOOK_DISABLED";

/// A failure of a poll.
enum Failure {
    /// The server cannot poll.
    Server(Error),
    /// The provider refused the calls, or failed.
    Provider(ProviderError),
}

/// Polls the link of a project, when it and its connection are active.
pub(super) async fn poll(ctx: &IntegrationContext, project: ProjectId) -> Result<(), Error> {
    // Read the link, its connection and what the last poll saw.
    let mut tx = ctx.pool.begin().await?;
    let linked = links::find(&mut tx, project).await?;
    let id = linked.as_ref().map(|linked| linked.link.connection_id);
    let connection = match id {
        Some(id) => connections::find(&mut tx, id).await?,
        None => None,
    };
    let seen = Seen::of(heads::read(&mut tx, project).await?);
    let hook = modes::hook_id(&mut tx, project).await?;
    tx.commit().await?;
    let (Some(linked), Some(connection)) = (linked, connection) else {
        return Ok(());
    };
    if linked.link.state != ConnectionState::Active {
        return Ok(());
    }

    // Read the heads on the provider.
    let connection = connection.connection;
    let read = match linked.provider {
        VcsProvider::GitHub => from_github(ctx, &linked, &connection, &seen).await,
        VcsProvider::GitLab => from_gitlab(ctx, &linked, hook.as_deref(), &seen).await,
    };
    let (listed, found) = match read {
        Ok(read) => read,
        Err(Failure::Server(error)) => return Err(error),
        Err(Failure::Provider(error)) => return delay(ctx, project, error).await,
    };

    // Keep the heads, then give each new head to the path of a webhook.
    keep(ctx, &linked, &listed, &seen).await?;
    let missed = !found.is_empty();
    for head in found {
        deliver::deliver(ctx, &linked, &connection, &seen, head).await?;
    }

    // Move a link that missed a webhook to the polling mode.
    let now = OffsetDateTime::now_utc();
    let quiet = linked.link.last_event_at.is_none_or(|at| at < now - WINDOW);
    if missed && quiet && linked.link.delivery_mode == DeliveryMode::Webhook {
        let reason = WEBHOOKS_NOT_ARRIVING;
        to_polling(ctx, project, reason, linked.provider == VcsProvider::GitHub).await?;
    }
    Ok(())
}

/// Reads the heads of a GitHub repository, and the commit of each new head of a branch.
async fn from_github(
    ctx: &IntegrationContext,
    linked: &Linked,
    connection: &VcsConnection,
    seen: &Seen,
) -> Result<(Listed, Vec<Head>), Failure> {
    // Read the heads with the App of the connection.
    let app = credentials::app_of(ctx, connection).await;
    let app = app.map_err(Failure::Server)?;
    let number = |text: &str| text.parse().map_err(|_| Failure::Server(not_a_number()));
    let repository = github::Repository {
        app: &app,
        installation: number(&connection.external_id)?,
        id: number(&linked.link.external_id)?,
        full_name: &linked.link.full_name,
    };
    let (branches, pulls) = (seen.cursor(BRANCHES).etag, seen.cursor(PULLS).etag);
    let listed = github::list(ctx, &repository, branches.as_deref(), pulls.as_deref()).await;
    let listed = listed.map_err(Failure::Provider)?;

    // Read the message and the author of each new head of a branch.
    let mut found = new_heads(seen, &listed, VcsProvider::GitHub);
    for head in &mut found {
        if matches!(head.kind, HeadKind::Branch(_)) {
            let commit = github::commit(ctx, &repository, &head.commit.sha).await;
            head.commit = commit.map_err(Failure::Provider)?;
        }
    }
    Ok((listed, found))
}

/// Reads the heads of a GitLab project. In webhook mode, a webhook that GitLab disabled moves
/// the link to the polling mode first.
async fn from_gitlab(
    ctx: &IntegrationContext,
    linked: &Linked,
    hook: Option<&str>,
    seen: &Seen,
) -> Result<(Listed, Vec<Head>), Failure> {
    // Check the webhook in webhook mode.
    let token = credentials::gitlab_token(ctx, linked.link.connection_id).await;
    let token = token.map_err(Failure::Server)?;
    let project = &linked.link.external_id;
    if let (Some(hook), DeliveryMode::Webhook) = (hook, linked.link.delivery_mode) {
        let works = gitlab::hook_works(ctx, &token, project, hook).await;
        if !works.map_err(Failure::Provider)? {
            let id = linked.link.project_id;
            to_polling(ctx, id, WEBHOOK_DISABLED, false)
                .await
                .map_err(Failure::Server)?;
        }
    }

    // Read the heads.
    let after = seen
        .cursor(PULLS)
        .updated_after
        .filter(|_| seen.knows(PULLS));
    let started = OffsetDateTime::now_utc();
    let listed = gitlab::list(ctx, &token, project, after, started).await;
    let listed = listed.map_err(Failure::Provider)?;
    let found = new_heads(seen, &listed, VcsProvider::GitLab);
    Ok((listed, found))
}

/// Returns the heads that changed since the last poll, in the lists that a poll read before.
fn new_heads(seen: &Seen, listed: &Listed, provider: VcsProvider) -> Vec<Head> {
    let lists = [(BRANCHES, &listed.branches), (PULLS, &listed.pulls)];
    lists
        .into_iter()
        .filter(|(name, _)| seen.knows(name))
        .filter_map(|(_, heads)| heads.as_ref())
        .flatten()
        .filter(|head| seen.is_new(&head.git_ref(provider), &head.commit.sha))
        .cloned()
        .collect()
}

/// Keeps the new heads and the cursors of a poll, and forgets the refs that a full list does not
/// have.
async fn keep(
    ctx: &IntegrationContext,
    linked: &Linked,
    listed: &Listed,
    seen: &Seen,
) -> Result<(), Error> {
    // Find the lists and the prefix of their refs.
    let (project, provider) = (linked.link.project_id, linked.provider);
    let pulls_prefix = match provider {
        VcsProvider::GitHub => "refs/pull/",
        VcsProvider::GitLab => "refs/merge-requests/",
    };
    let lists = [
        (
            BRANCHES,
            &listed.branches,
            true,
            &listed.branches_cursor,
            "refs/heads/",
        ),
        (
            PULLS,
            &listed.pulls,
            listed.all_pulls,
            &listed.pulls_cursor,
            pulls_prefix,
        ),
    ];

    // Keep the new heads and the cursor of each list.
    let mut tx = ctx.pool.begin().await?;
    for (name, found, full, cursor, prefix) in lists {
        if let Some(found) = found {
            let refs: Vec<String> = found.iter().map(|head| head.git_ref(provider)).collect();
            for (head, git_ref) in found.iter().zip(&refs) {
                if seen.is_new(git_ref, &head.commit.sha) {
                    heads::keep(&mut tx, project, git_ref, &head.commit.sha).await?;
                }
            }
            if full {
                heads::forget_others(&mut tx, project, prefix, &refs).await?;
            }
        }
        let etag = cursor.etag.as_deref();
        heads::cursor(&mut tx, project, name, etag, cursor.updated_after).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Moves a link in webhook mode to the polling mode with a reason, and its GitHub connection
/// too: the webhooks of one App reach the server or not.
pub(crate) async fn to_polling(
    ctx: &IntegrationContext,
    project: ProjectId,
    reason: &str,
    github: bool,
) -> Result<(), Error> {
    let mut tx = ctx.pool.begin().await?;
    let moved = modes::to_polling(&mut tx, project, reason).await?;
    if let (Some(connection), true) = (moved, github) {
        modes::connection_to_polling(&mut tx, connection).await?;
    }
    tx.commit().await?;
    if moved.is_some() {
        tracing::info!(%project, reason, "a repository link moved to the polling mode");
    }
    Ok(())
}

/// Logs a failed call of a poll. A limit of the provider moves the next poll to its end.
async fn delay(
    ctx: &IntegrationContext,
    project: ProjectId,
    error: ProviderError,
) -> Result<(), Error> {
    tracing::warn!(%project, %error, "a poll of a repository failed");
    if let ProviderError::RateLimited(until) = error {
        let mut tx = ctx.pool.begin().await?;
        modes::delay(&mut tx, project, until).await?;
        tx.commit().await?;
    }
    Ok(())
}

/// The error of an id of GitHub in a row that is not a number.
fn not_a_number() -> Error {
    Error::Internal("an id of GitHub is not a number".to_owned())
}
