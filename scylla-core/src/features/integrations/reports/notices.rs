//! The notices of the health of a connection: a connection that needs a person, and a GitLab
//! token that expires soon. The owners and the admins of the workspace get them.

use std::sync::Arc;

use scylla_domain::authz::Object;
use scylla_domain::integrations::connections::VcsConnectionHolder;
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_extension::{
    async_trait, Access, Audience, AudienceContext, Collapse, Event, EventRow, NotificationError,
    NotificationKind, Recipient,
};

use super::store::people;
use crate::features::integrations::connections::store::health::Changed;
use crate::Error;

/// The key `vcs.connection_broken`.
pub(crate) const CONNECTION_BROKEN: &str = "vcs.connection_broken";
/// The key `vcs.token_expiring`.
pub(crate) const TOKEN_EXPIRING: &str = "vcs.token_expiring";

/// The category of the facts of the Git integration.
const INTEGRATIONS: &str = "integrations";
/// The kind of the object of a fact about a connection.
const CONNECTION: &str = "vcs_connection";
/// The reason of an owner of the workspace.
const OWNER: &str = "owner";
/// The reason of an admin of the workspace.
const WORKSPACE_ADMIN: &str = "workspace_admin";

/// Returns the kinds of the notices of the connections.
pub(crate) fn kinds() -> Vec<NotificationKind> {
    let access = Access::Holds(VcsConnectionHolder::GET_VCS_CONNECTION.name());
    [CONNECTION_BROKEN, TOKEN_EXPIRING]
        .into_iter()
        .map(|key| {
            NotificationKind::new(key, INTEGRATIONS, Arc::new(OwnersAndAdmins))
                .access(access)
                .collapse(Collapse::Replace)
        })
        .collect()
}

/// Makes the fact of a connection that needs a person, with the reason.
pub(super) fn broken(connection: &Changed, reason: &str) -> Event {
    about(CONNECTION_BROKEN, connection).param("reason", reason)
}

/// Makes the fact of a token that expires soon, with its day of expiry.
pub(super) fn expiring(connection: &Changed) -> Event {
    let date = connection
        .expires_at
        .map(|at| {
            format!(
                "{:04}-{:02}-{:02}",
                at.year(),
                u8::from(at.month()),
                at.day()
            )
        })
        .unwrap_or_default();
    let dedupe = format!("{}:{}:{date}", TOKEN_EXPIRING, connection.id);
    about(TOKEN_EXPIRING, connection)
        .param("expires_at", date)
        .dedupe(dedupe)
}

/// Makes a fact about a connection, in its workspace.
fn about(kind: &str, connection: &Changed) -> Event {
    Event::new(kind)
        .subject(CONNECTION, connection.id)
        .workspace(connection.workspace)
        .param("provider", connection.provider.clone())
        .param("account", connection.account.clone())
        .group(format!("{kind}:{}", connection.id))
}

/// The owners and the admins of the workspace of a fact.
struct OwnersAndAdmins;

#[async_trait]
impl Audience for OwnersAndAdmins {
    async fn recipients(
        &self,
        ctx: &AudienceContext<'_>,
        event: &EventRow,
    ) -> Result<Vec<Recipient>, NotificationError> {
        // Read the owners of the workspace.
        let failed = |error: Error| NotificationError(error.to_string());
        let Some(workspace) = event.event.workspace else {
            return Ok(Vec::new());
        };
        let mut tx = ctx.pool.begin().await.map_err(|e| failed(e.into()))?;
        let owners = people::owners(&mut tx, workspace).await.map_err(failed)?;
        tx.commit().await.map_err(|e| failed(e.into()))?;

        // Add the admins of the workspace that are not owners.
        let roles = ctx.gate.all_roles(&Object::of(workspace)).await;
        let roles = roles.map_err(|error| NotificationError(error.to_string()))?;
        let admins = roles
            .into_iter()
            .filter(|holder| holder.role == Workspace::ADMIN.name())
            .map(|holder| holder.actor)
            .filter(|actor| !owners.contains(actor));
        let mut recipients: Vec<Recipient> = owners
            .iter()
            .map(|owner| Recipient::new(*owner, OWNER))
            .collect();
        for admin in admins {
            if recipients.iter().all(|known| known.user != admin) {
                recipients.push(Recipient::new(admin, WORKSPACE_ADMIN));
            }
        }
        Ok(recipients)
    }
}
