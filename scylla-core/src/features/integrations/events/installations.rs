//! The changes of an installation of the GitHub App that GitHub tells: an uninstall, a suspend
//! and its end, new permissions, and the repositories that the installation sees. They change
//! the connections of the installation and their links. The owners and the admins of a
//! workspace get a notice when its connection stops.

use std::collections::BTreeMap;

use scylla_domain::integrations::apps::VcsAppId;
use scylla_domain::integrations::connections::ConnectionState;
use scylla_domain::projects::ProjectId;
use scylla_extension::Tx;

use crate::features::integrations::connections::checks;
use crate::features::integrations::connections::store::installations::{self, StateChange};
use crate::features::integrations::reports::notices;
use crate::features::integrations::IntegrationContext;
use crate::features::notifications;
use crate::Error;

/// The reason of a connection whose App was uninstalled.
const APP_UNINSTALLED: &str = "APP_UNINSTALLED";

/// The reason of a connection whose App an owner suspended.
const APP_SUSPENDED: &str = "APP_SUSPENDED";

/// A change of an installation of the GitHub App.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InstallationChange {
    /// An owner uninstalled the App.
    Deleted {
        /// The id of the installation.
        installation: String,
    },
    /// An owner suspended the App.
    Suspended {
        /// The id of the installation.
        installation: String,
    },
    /// An owner started the App again.
    Unsuspended {
        /// The id of the installation.
        installation: String,
    },
    /// An owner accepted new permissions.
    Permissions {
        /// The id of the installation.
        installation: String,
        /// The permissions of the installation now, by name.
        permissions: BTreeMap<String, String>,
    },
    /// An owner added repositories to the installation, or removed some.
    Repositories {
        /// The id of the installation.
        installation: String,
        /// `true` when the installation sees all the repositories of its account.
        all: bool,
        /// The ids of the repositories that the installation now sees.
        added: Vec<String>,
        /// The ids of the repositories that the installation no longer sees.
        removed: Vec<String>,
    },
}

/// The new state of the connections of a change, and its reason.
type Target = (ConnectionState, Option<&'static str>);

/// Applies a change of an installation of an App to its connections and their links.
pub(super) async fn apply(
    ctx: &IntegrationContext,
    app: Option<VcsAppId>,
    change: &InstallationChange,
) -> Result<(), Error> {
    // Change the connections, or the links of the repositories.
    let mut tx = ctx.pool.begin().await?;
    let (changed, target) = match change {
        InstallationChange::Repositories {
            installation,
            all,
            added,
            removed,
        } => {
            let counts = (added.len(), removed.len());
            installations::set_repositories(&mut tx, app, installation, *all, counts).await?;
            let (stopped, restored) =
                installations::set_visible(&mut tx, app, installation, added, removed).await?;
            log_links(installation, &stopped, &restored);
            (Vec::new(), (ConnectionState::Active, None))
        }
        change => connections(ctx, &mut tx, app, change).await?,
    };

    // Tell the owners and the admins about each connection that stopped.
    let (state, reason) = target;
    if state != ConnectionState::Active {
        for change in &changed {
            let notice = notices::broken(&change.changed, reason.unwrap_or_default());
            notifications::emit(&mut tx, notice).await?;
        }
    }
    tx.commit().await?;

    // Log each change for the operators.
    for change in &changed {
        tracing::info!(
            event = "vcs.connection_state_changed",
            connection = %change.changed.id,
            workspace = %change.changed.workspace,
            from = change.from,
            to = state.as_str(),
            reason = reason.unwrap_or_default(),
        );
    }
    Ok(())
}

/// Changes the state of the connections of an installation: disconnected after an uninstall,
/// suspended and back, or the state that new permissions give. Returns the connections that
/// changed and their new state.
async fn connections(
    ctx: &IntegrationContext,
    tx: &mut Tx<'_>,
    app: Option<VcsAppId>,
    change: &InstallationChange,
) -> Result<(Vec<StateChange>, Target), Error> {
    // Find the installation, the new state and the states that can move to it.
    let (active, suspended, attention) = (
        ConnectionState::Active,
        ConnectionState::Suspended,
        ConnectionState::NeedsAttention,
    );
    let (installation, target, from) = match change {
        InstallationChange::Deleted { installation } => {
            forget(ctx, app, installation);
            let target = (ConnectionState::Disconnected, Some(APP_UNINSTALLED));
            (installation, target, vec![active, suspended, attention])
        }
        InstallationChange::Suspended { installation } => {
            forget(ctx, app, installation);
            let target = (suspended, Some(APP_SUSPENDED));
            (installation, target, vec![active, attention])
        }
        InstallationChange::Unsuspended { installation } => {
            (installation, (active, None), vec![suspended])
        }
        InstallationChange::Permissions {
            installation,
            permissions,
        } => {
            let missing = checks::missing_permissions(permissions);
            let target = checks::installation_state(false, &missing);
            let json = serde_json::to_value(permissions)
                .map_err(|error| Error::Internal(format!("the permissions of GitHub: {error}")))?;
            let changed =
                installations::set_permissions(tx, app, installation, &json, target).await?;
            return Ok((changed, target));
        }
        InstallationChange::Repositories { .. } => return Ok((Vec::new(), (active, None))),
    };

    // Move the connections.
    let changed = installations::set_state(tx, app, installation, target, &from).await?;
    Ok((changed, target))
}

/// Logs the links whose repository the installation stopped or started to see.
fn log_links(installation: &str, stopped: &[ProjectId], restored: &[ProjectId]) {
    for project in stopped {
        tracing::info!(%project, installation, "a linked repository is not visible");
    }
    for project in restored {
        tracing::info!(%project, installation, "a linked repository is visible again");
    }
}

/// Drops the tokens of an installation of the App of the settings from its cache.
fn forget(ctx: &IntegrationContext, app: Option<VcsAppId>, installation: &str) {
    let (Some(github), None, Ok(id)) = (&ctx.clients.github_app, app, installation.parse()) else {
        return;
    };
    github.forget(id);
}
