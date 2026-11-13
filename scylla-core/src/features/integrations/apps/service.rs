//! The read and the delete of the GitHub App of the instance, and the refusals of the routes that
//! change it.

use scylla_domain::integrations::apps::{VcsApp, VcsAppId};
use scylla_domain::integrations::connections::ConnectionState;
use scylla_extension::Rejection;

use super::store::{self, connections, deliveries, deliveries::LastDelivery};
use crate::features::integrations::reports::notices;
use crate::features::integrations::IntegrationContext;
use crate::features::notifications;
use crate::Error;

/// The reason of a connection whose App an admin removed: as for an uninstall, the connection
/// needs a new install.
const APP_UNINSTALLED: &str = "APP_UNINSTALLED";

/// The GitHub App that the server uses.
pub(super) enum Found {
    /// The App of the settings of the server, with its id and its slug on GitHub.
    Settings(u64, String),
    /// The App of the instance, made with the manifest flow.
    Instance(VcsApp),
}

/// The GitHub App that the server uses, and its last webhook.
pub(super) struct Shown {
    /// The App.
    pub(super) found: Found,
    /// The last webhook, or `None` before the first one.
    pub(super) last: Option<LastDelivery>,
}

/// Returns the GitHub App that the server uses: the App of the settings, else the App of the
/// instance. `None` when the server has no App.
pub(super) async fn get(ctx: &IntegrationContext) -> Result<Option<Shown>, Error> {
    let mut tx = ctx.pool.begin().await?;
    let found = match &ctx.clients.github_app {
        Some(app) => {
            let settings = app.settings();
            Some(Found::Settings(settings.app_id, settings.slug.clone()))
        }
        None => store::find(&mut tx).await?.map(Found::Instance),
    };
    let shown = match found {
        Some(found) => {
            let last = deliveries::last(&mut tx, found.row()).await?;
            Some(Shown { found, last })
        }
        None => None,
    };
    tx.commit().await?;
    Ok(shown)
}

/// Removes the GitHub App of the instance. Each of its connections becomes disconnected, and the
/// owners and the admins of its workspace get a notice. An instance with no App gives
/// `NOT_FOUND`.
pub(super) async fn delete(ctx: &IntegrationContext) -> Result<(), Error> {
    // Lock the App, and disconnect its connections.
    let mut tx = ctx.pool.begin().await?;
    let id = store::lock(&mut tx)
        .await?
        .ok_or(Error::NotFound("GitHub App"))?;
    let changed = connections::disconnect(&mut tx, id, APP_UNINSTALLED).await?;
    let disconnected = ConnectionState::Disconnected.as_str();
    let stopped: Vec<_> = changed
        .iter()
        .filter(|change| change.from != disconnected)
        .collect();

    // Tell the owners and the admins about each connection that stopped, then delete the App.
    for change in &stopped {
        let notice = notices::broken(&change.changed, APP_UNINSTALLED);
        notifications::emit(&mut tx, notice).await?;
    }
    store::delete(&mut tx, id).await?;
    tx.commit().await?;

    // Log each change for the operators.
    for change in &stopped {
        tracing::info!(
            event = "vcs.connection_state_changed",
            connection = %change.changed.id,
            workspace = %change.changed.workspace,
            from = change.from,
            to = disconnected,
            reason = APP_UNINSTALLED,
        );
    }
    tracing::info!(event = "vcs.app_deleted", app = %id, connections = changed.len());
    Ok(())
}

/// Refuses a change when the App comes from the settings of the server.
pub(super) fn refuse_settings(ctx: &IntegrationContext) -> Result<(), Error> {
    match ctx.clients.github_app {
        Some(_) => {
            let message = "the GitHub App comes from the settings of the server";
            Err(Error::Rejected(Rejection::new(
                "APP_MANAGED_BY_SETTINGS",
                message,
            )))
        }
        None => Ok(()),
    }
}

/// The refusal of a second App on the instance.
pub(super) fn app_exists() -> Error {
    let message = "the instance has a GitHub App: remove it first";
    Error::Rejected(Rejection::new("APP_EXISTS", message))
}

impl Found {
    /// Returns the id of the row of the App of the instance, or `None` for the App of the
    /// settings.
    pub(super) fn row(&self) -> Option<VcsAppId> {
        match self {
            Self::Settings(..) => None,
            Self::Instance(app) => Some(app.id),
        }
    }
}
