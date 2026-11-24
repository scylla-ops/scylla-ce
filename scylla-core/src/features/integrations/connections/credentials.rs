//! The credentials of the connections: the GitHub App of the settings or of the instance, the
//! sealed token of a GitLab connection, and the refusals of a missing App, a missing key and a
//! failed call to a provider.

use std::sync::Arc;

use scylla_domain::integrations::apps::VcsAppId;
use scylla_domain::integrations::connections::{VcsConnection, VcsConnectionId};
use scylla_domain::integrations::VcsProvider;
use scylla_extension::Rejection;

use crate::features::integrations::apps::store as apps;
use crate::features::integrations::apps::store::SealedApp;
use crate::features::integrations::IntegrationContext;
use crate::infra::secret::Sealer;
use crate::infra::vcs::{GitHubApp, GitHubAppSettings, Hidden, ProviderError};
use crate::Error;

use super::store;

/// The GitHub App that a connection uses, and its row when it is the App of the instance.
pub(crate) struct AppSource {
    /// The App, with its key and its tokens.
    pub(crate) app: Arc<GitHubApp>,
    /// The row of the App of the instance, or `None` for the App of the settings.
    pub(crate) row: Option<VcsAppId>,
}

/// Returns the GitHub App of a new connection: the App of the settings, else the App of the
/// instance. `None` when the server has no App.
pub(crate) async fn github_app(ctx: &IntegrationContext) -> Result<Option<AppSource>, Error> {
    // Take the App of the settings.
    if let Some(app) = &ctx.clients.github_app {
        let app = Arc::clone(app);
        return Ok(Some(AppSource { app, row: None }));
    }

    // Else open the App of the instance.
    let mut tx = ctx.pool.begin().await?;
    let row = apps::github(&mut tx).await?;
    tx.commit().await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let id = row.id;
    let app = open_app(ctx, row)?;
    Ok(Some(AppSource {
        app: Arc::new(app),
        row: Some(id),
    }))
}

/// Returns the GitHub App of a connection: the App of the instance of its row, else the App of
/// the settings. A missing App gives `APP_MISSING`.
pub(crate) async fn app_of(
    ctx: &IntegrationContext,
    connection: &VcsConnection,
) -> Result<Arc<GitHubApp>, Error> {
    let source = github_app(ctx).await?.ok_or_else(app_missing)?;
    if source.row != connection.app_id && connection.app_id.is_some() {
        return Err(app_missing());
    }
    Ok(source.app)
}

/// Opens the token of a GitLab connection. A server with no key gives `SECRET_KEY_MISSING`.
pub(crate) async fn gitlab_token(
    ctx: &IntegrationContext,
    id: VcsConnectionId,
) -> Result<Hidden, Error> {
    let sealer = key(ctx.sealer.as_deref())?;
    let mut tx = ctx.pool.begin().await?;
    let sealed = store::sealed_token(&mut tx, id).await?;
    tx.commit().await?;
    let sealed = sealed.ok_or(Error::NotFound("GitLab token"))?;
    open(sealer, &sealed, &id.to_bytes(), "the token of a connection").map(Hidden::new)
}

/// Returns the key of the secrets, or `SECRET_KEY_MISSING` if the server has none.
pub(crate) fn key(sealer: Option<&Sealer>) -> Result<&Sealer, Error> {
    sealer.ok_or_else(|| {
        let message = "the server has no key for the secrets: set SCYLLA_SECRET_KEY";
        Error::Rejected(Rejection::new("SECRET_KEY_MISSING", message))
    })
}

/// The refusal of an instance with no GitHub App.
pub(crate) fn app_missing() -> Error {
    let message = "the instance has no GitHub App";
    Error::Rejected(Rejection::new("APP_MISSING", message))
}

/// Returns the refusal of a failed call to a provider: `PROVIDER_REFUSED` when the provider
/// refused the credentials, else `PROVIDER_UNAVAILABLE`, with the end of a limit.
pub(crate) fn provider_failed(provider: VcsProvider, error: ProviderError) -> Error {
    let name = match provider {
        VcsProvider::GitHub => "GitHub",
        VcsProvider::GitLab => "GitLab",
    };
    let message = format!("{name}: {error}");
    let rejection = match error {
        ProviderError::Unauthorized | ProviderError::Forbidden | ProviderError::NotFound => {
            Rejection::new("PROVIDER_REFUSED", message)
        }
        ProviderError::RateLimited(until) => Rejection::new("PROVIDER_UNAVAILABLE", message)
            .detail("retry_at", until.unix_timestamp()),
        ProviderError::Invalid(_) | ProviderError::Transient(_) => {
            Rejection::new("PROVIDER_UNAVAILABLE", message)
        }
    };
    Error::Rejected(rejection)
}

/// Opens the secrets of the App of the instance with the id of its row, and makes the App.
fn open_app(ctx: &IntegrationContext, row: SealedApp) -> Result<GitHubApp, Error> {
    let sealer = key(ctx.sealer.as_deref())?;
    let data = row.id.to_bytes();
    let settings = GitHubAppSettings {
        app_id: u64::try_from(row.app_id)
            .map_err(|_| Error::Internal("the id of the GitHub App is negative".to_owned()))?,
        slug: row.slug,
        client_id: row.client_id,
        client_secret: Hidden::new(open(sealer, &row.client_secret, &data, "a client secret")?),
        private_key: Hidden::new(open(sealer, &row.private_key, &data, "a private key")?),
        webhook_secret: Hidden::new(open(sealer, &row.webhook_secret, &data, "a secret")?),
    };
    ctx.clients
        .app(&settings)
        .map_err(|reason| Error::Internal(format!("the GitHub App of the instance {reason}")))
}

/// Opens a sealed text with its associated data. The error names the value, never its text.
fn open(sealer: &Sealer, sealed: &[u8], data: &[u8], what: &str) -> Result<String, Error> {
    let plain = sealer
        .open_with(sealed, data)
        .map_err(|reason| Error::Internal(format!("{what} does not open: {reason}")))?;
    String::from_utf8(plain).map_err(|_| Error::Internal(format!("{what} is not UTF-8")))
}
