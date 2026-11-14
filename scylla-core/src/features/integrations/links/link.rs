//! The link of a project to a repository: the check that the connection sees the repository,
//! the webhook of the GitHub App or of a GitLab project, then the row of the link.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::integrations::connections::{DeliveryMode, VcsConnection};
use scylla_domain::integrations::links::{GitLabRole, ReportMode};
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::ProjectId;
use scylla_extension::Rejection;

use super::calls::{github, gitlab};
use super::service;
use super::store::{self, Linked, NewLink, Repository};
use crate::features::integrations::connections::calls::github as connections_github;
use crate::features::integrations::connections::{checks, credentials};
use crate::features::integrations::polling::{self, store as polls};
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::{gitlab_signing_token, Hidden, ProviderError};
use crate::Error;

/// The reason of a GitLab link with no webhook: the service account is only Developer.
const ROLE_TOO_LOW: &str = "ROLE_TOO_LOW";

/// How GitLab tells Scylla about the new commits of a project.
enum Delivery {
    /// A webhook, with its id and its signing token.
    Webhook(String, Hidden),
    /// No webhook: Scylla polls, for this reason.
    Polling(Option<&'static str>),
}

/// What the provider gave for a new link: the repository, and how the provider tells Scylla
/// about the new commits.
struct Found {
    /// The repository.
    repository: Repository,
    /// How Scylla reports.
    report_mode: ReportMode,
    /// The delivery mode.
    delivery_mode: DeliveryMode,
    /// The GitLab webhook and its signing token, or `None`.
    hook: Option<(String, Hidden)>,
    /// The reason of the delivery mode, or `None`.
    reason: Option<&'static str>,
}

/// Links a project to a repository of a connection of its workspace. A project with a link gives
/// `REPOSITORY_ALREADY_LINKED`, and a repository that the connection cannot see gives
/// `REPOSITORY_NOT_VISIBLE`. On GitHub, an App with no webhook gives the polling mode. On GitLab,
/// a Maintainer gets a webhook, and a Developer gets the polling mode.
pub(super) async fn link(
    ctx: &IntegrationContext,
    project: ProjectId,
    connection: &VcsConnection,
    external_id: &str,
    caller: ActorId,
) -> Result<Linked, Error> {
    // Refuse a project with a link before a call to the provider.
    let mut tx = ctx.pool.begin().await?;
    let exists = store::exists(&mut tx, project).await?;
    tx.commit().await?;
    if exists {
        return Err(already_linked());
    }

    // Find the repository, and make the webhook of a GitLab project.
    let found = match connection.provider {
        VcsProvider::GitHub => github(ctx, connection, external_id).await?,
        VcsProvider::GitLab => gitlab(ctx, connection, project, external_id).await?,
    };

    // Seal the signing token with the id of the project, and keep the link.
    let sealed = match &found.hook {
        Some((hook, token)) => {
            let sealer = credentials::key(ctx.sealer.as_deref())?;
            Some((
                hook.as_str(),
                sealer.seal_with(token.expose().as_bytes(), &project.to_bytes()),
            ))
        }
        None => None,
    };
    let new = NewLink {
        project,
        workspace: connection.workspace_id,
        connection: connection.id,
        repository: &found.repository,
        report_mode: found.report_mode,
        delivery_mode: found.delivery_mode,
        poll_interval_seconds: connection.poll_interval_seconds,
        hook: sealed
            .as_ref()
            .map(|(hook, sealed)| (*hook, sealed.as_slice())),
        reason: found.reason,
        created_by: caller,
    };

    // A failed save, or a link that another call made first, removes the new webhook.
    let linked = match save(ctx, &new).await {
        Ok(Some(linked)) => linked,
        failed => {
            if let Some((hook, _)) = &found.hook {
                let id = &found.repository.external_id;
                service::delete_hook(ctx, connection.id, id, hook).await;
            }
            return Err(failed.err().unwrap_or_else(already_linked));
        }
    };
    tracing::info!(
        event = "repository.linked",
        %project,
        workspace = %connection.workspace_id,
        connection = %connection.id,
        provider = connection.provider.as_str(),
        repository = %found.repository.full_name,
        delivery = found.delivery_mode.as_str(),
    );
    Ok(linked)
}

/// Inserts a link, and reads it back. Returns `None` when the project has a link. A link of a
/// GitHub App with no webhook moves its connection to the polling mode too.
async fn save(ctx: &IntegrationContext, new: &NewLink<'_>) -> Result<Option<Linked>, Error> {
    let mut tx = ctx.pool.begin().await?;
    if !store::insert(&mut tx, new).await? {
        return Ok(None);
    }
    if new.reason == Some(polling::APP_HAS_NO_WEBHOOK) {
        polls::links::connection_to_polling(&mut tx, new.connection).await?;
    }
    let linked = store::find(&mut tx, new.project).await?;
    tx.commit().await?;
    Ok(linked)
}

/// Finds a repository of a GitHub installation. The link polls at once when the App has no
/// webhook, else it takes the delivery mode of the connection.
async fn github(
    ctx: &IntegrationContext,
    connection: &VcsConnection,
    external_id: &str,
) -> Result<Found, Error> {
    // Read the repositories of the installation.
    let app = credentials::app_of(ctx, connection).await?;
    let installation = connection
        .external_id
        .parse()
        .map_err(|_| Error::Internal("an installation id is not a number".to_owned()))?;
    let repositories = connections_github::repositories(&ctx.clients, &app, installation).await;
    let repositories = repositories.map_err(|error| failed(connection, error))?;

    // Keep the repository of the request.
    let repository = repositories
        .into_iter()
        .find(|repository| repository.id.to_string() == external_id)
        .ok_or_else(not_visible)?;

    // Read the webhook of the App. A failed read keeps the mode of the connection.
    let (delivery_mode, reason) = match github::app_has_webhook(&ctx.clients, &app).await {
        Ok(false) => (DeliveryMode::Polling, Some(polling::APP_HAS_NO_WEBHOOK)),
        Ok(true) => (connection.delivery_mode, None),
        Err(error) => {
            let id = connection.id;
            tracing::warn!(connection = %id, %error, "the webhook of the GitHub App is unknown");
            (connection.delivery_mode, None)
        }
    };
    Ok(Found {
        repository: Repository {
            external_id: repository.id.to_string(),
            full_name: repository.full_name,
            web_url: repository.html_url,
            default_branch: repository.default_branch.unwrap_or_default(),
            private: repository.private,
            role: None,
        },
        report_mode: ReportMode::CheckRun,
        delivery_mode,
        hook: None,
        reason,
    })
}

/// Finds a GitLab project where the service account is Developer or more, and makes its webhook
/// when the account is Maintainer or more.
async fn gitlab(
    ctx: &IntegrationContext,
    connection: &VcsConnection,
    project: ProjectId,
    external_id: &str,
) -> Result<Found, Error> {
    // Read the project with the access of the service account.
    let token = credentials::gitlab_token(ctx, connection.id).await?;
    let found = match gitlab::project(&ctx.clients, &token, external_id).await {
        Ok(found) => found,
        Err(ProviderError::NotFound | ProviderError::Forbidden) => return Err(not_visible()),
        Err(error) => return Err(failed(connection, error)),
    };
    let role = checks::gitlab_role(found.level()).ok_or_else(not_visible)?;
    let repository = Repository {
        external_id: found.id.to_string(),
        private: found.private(),
        full_name: found.path_with_namespace,
        web_url: found.web_url,
        default_branch: found.default_branch.unwrap_or_default(),
        role: Some(role),
    };

    // Make the webhook. A Developer, or a refusal of GitLab, gives the polling mode.
    let delivery = match role {
        GitLabRole::Developer => Delivery::Polling(Some(ROLE_TOO_LOW)),
        GitLabRole::Maintainer | GitLabRole::Owner => {
            hook(ctx, connection, project, &repository, &token).await?
        }
    };
    let (delivery_mode, hook, reason) = match delivery {
        Delivery::Webhook(id, signing_token) => {
            (DeliveryMode::Webhook, Some((id, signing_token)), None)
        }
        Delivery::Polling(reason) => (DeliveryMode::Polling, None, reason),
    };
    Ok(Found {
        repository,
        report_mode: ReportMode::CommitStatus,
        delivery_mode,
        hook,
        reason,
    })
}

/// Makes the webhook of a GitLab project with a new signing token. GitLab can refuse it: a
/// refused role gives the polling mode with `ROLE_TOO_LOW`, and a URL that GitLab does not
/// accept gives the polling mode with no reason.
async fn hook(
    ctx: &IntegrationContext,
    connection: &VcsConnection,
    project: ProjectId,
    repository: &Repository,
    token: &Hidden,
) -> Result<Delivery, Error> {
    let url = format!("{}/integrations/gitlab/{project}/webhook", ctx.urls.api_url);
    let signing_token = gitlab_signing_token();
    let id = &repository.external_id;
    match gitlab::create_hook(&ctx.clients, token, id, &url, &signing_token).await {
        Ok(hook) => Ok(Delivery::Webhook(hook, signing_token)),
        Err(ProviderError::Forbidden) => Ok(Delivery::Polling(Some(ROLE_TOO_LOW))),
        Err(ProviderError::Invalid(reason)) => {
            tracing::warn!(%project, %reason, "GitLab refused the webhook: the link polls");
            Ok(Delivery::Polling(None))
        }
        Err(error) => Err(failed(connection, error)),
    }
}

/// The refusal of a project with a link.
fn already_linked() -> Error {
    let message = "the project has a linked repository: unlink it first";
    Error::Rejected(Rejection::new("REPOSITORY_ALREADY_LINKED", message))
}

/// The refusal of a repository that the connection cannot see.
fn not_visible() -> Error {
    let message = "the connection cannot see this repository";
    Error::Rejected(Rejection::new("REPOSITORY_NOT_VISIBLE", message))
}

/// Returns the refusal of a failed call to the provider of a connection.
fn failed(connection: &VcsConnection, error: ProviderError) -> Error {
    credentials::provider_failed(connection.provider, error)
}
