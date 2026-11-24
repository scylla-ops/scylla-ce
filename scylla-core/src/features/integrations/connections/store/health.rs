//! The SQL of the health of the connections: a connection that needs a person after a refused
//! call or an expired token, and the GitLab tokens that expire soon.

use scylla_domain::integrations::connections::VcsConnectionId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;
use time::OffsetDateTime;

use crate::Error;

/// A connection whose health changed.
pub(crate) struct Changed {
    /// The connection.
    pub(crate) id: VcsConnectionId,
    /// Its workspace.
    pub(crate) workspace: WorkspaceId,
    /// Its provider.
    pub(crate) provider: String,
    /// The login of its account.
    pub(crate) account: String,
    /// The expiry of its token.
    pub(crate) expires_at: Option<OffsetDateTime>,
}

/// Moves an active connection to `needs_attention` with a reason. Returns the connection, or
/// `None` when it was not active.
pub(crate) async fn needs_attention(
    tx: &mut Tx<'_>,
    id: VcsConnectionId,
    reason: &str,
) -> Result<Option<Changed>, Error> {
    let row = sqlx::query!(
        "UPDATE vcs_connections
         SET state = 'needs_attention', state_reason = $2, updated_at = now()
         WHERE id = $1 AND state = 'active'
         RETURNING workspace_id, provider, account, token_expires_at",
        id.to_db(),
        reason,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| Changed {
        id,
        workspace: WorkspaceId::from_db(row.workspace_id),
        provider: row.provider,
        account: row.account,
        expires_at: row.token_expires_at,
    }))
}

/// Moves each active connection whose token expired to `needs_attention` with the reason
/// `TOKEN_EXPIRED`, and returns them.
pub(crate) async fn expire(tx: &mut Tx<'_>) -> Result<Vec<Changed>, Error> {
    let rows = sqlx::query!(
        "UPDATE vcs_connections
         SET state = 'needs_attention', state_reason = 'TOKEN_EXPIRED', updated_at = now()
         WHERE token_expires_at <= now() AND state = 'active'
         RETURNING id, workspace_id, provider, account, token_expires_at",
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Changed {
            id: VcsConnectionId::from_db(row.id),
            workspace: WorkspaceId::from_db(row.workspace_id),
            provider: row.provider,
            account: row.account,
            expires_at: row.token_expires_at,
        })
        .collect())
}

/// Marks as warned each active connection whose token expires within `days` days and that got no
/// warning for this token, and returns them.
pub(crate) async fn warn_expiring(tx: &mut Tx<'_>, days: i32) -> Result<Vec<Changed>, Error> {
    let rows = sqlx::query!(
        "UPDATE vcs_connections
         SET token_warned_at = now()
         WHERE token_expires_at > now() AND token_expires_at <= now() + make_interval(days => $1)
           AND token_warned_at IS NULL AND state = 'active'
         RETURNING id, workspace_id, provider, account, token_expires_at",
        days,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Changed {
            id: VcsConnectionId::from_db(row.id),
            workspace: WorkspaceId::from_db(row.workspace_id),
            provider: row.provider,
            account: row.account,
            expires_at: row.token_expires_at,
        })
        .collect())
}
