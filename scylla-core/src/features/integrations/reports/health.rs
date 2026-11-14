//! The health of the connections: a connection that needs a person after a refused call or an
//! expired token, and the warning 14 days before a GitLab token expires. The owners and the
//! admins of the workspace get a notice in the transaction of the change.

use scylla_domain::integrations::connections::{ConnectionState, VcsConnectionId};
use scylla_extension::Tx;

use super::notices;
use crate::features::integrations::connections::store::health::{self, Changed};
use crate::features::integrations::IntegrationContext;
use crate::features::notifications;
use crate::Error;

/// The days before the expiry of a token when the owners get a warning.
const WARN_DAYS: i32 = 14;

/// The reason of a connection whose token expired.
const TOKEN_EXPIRED: &str = "TOKEN_EXPIRED";

/// Warns about the tokens that expire within 14 days, and moves the connections whose token
/// expired to `needs_attention`.
pub(super) async fn check(ctx: &IntegrationContext) -> Result<(), Error> {
    // Warn one time for each token that expires soon.
    let mut tx = ctx.pool.begin().await?;
    for connection in health::warn_expiring(&mut tx, WARN_DAYS).await? {
        notifications::emit(&mut tx, notices::expiring(&connection)).await?;
    }

    // Stop the connections whose token expired.
    let expired = health::expire(&mut tx).await?;
    for connection in &expired {
        notifications::emit(&mut tx, notices::broken(connection, TOKEN_EXPIRED)).await?;
    }
    tx.commit().await?;
    for connection in &expired {
        log_changed(connection, TOKEN_EXPIRED);
    }
    Ok(())
}

/// Moves an active connection to `needs_attention` after a refused call, and records the notice.
/// Returns the connection, or `None` when it was not active.
pub(super) async fn broken(
    tx: &mut Tx<'_>,
    connection: VcsConnectionId,
    reason: &'static str,
) -> Result<Option<(Changed, &'static str)>, Error> {
    let Some(changed) = health::needs_attention(tx, connection, reason).await? else {
        return Ok(None);
    };
    notifications::emit(tx, notices::broken(&changed, reason)).await?;
    Ok(Some((changed, reason)))
}

/// Logs the change of a connection after a refused call.
pub(super) fn log_broken(broken: &(Changed, &'static str)) {
    log_changed(&broken.0, broken.1);
}

/// Logs the change of the state of a connection to `needs_attention`.
fn log_changed(connection: &Changed, reason: &str) {
    tracing::info!(
        event = "vcs.connection_state_changed",
        connection = %connection.id,
        workspace = %connection.workspace,
        from = ConnectionState::Active.as_str(),
        to = ConnectionState::NeedsAttention.as_str(),
        reason,
    );
}
