//! The SQL of the changes of an installation of the GitHub App that its webhooks tell: the state
//! and the permissions of its connections, the repositories that it sees, and the state of the
//! links of these repositories. The connections of one installation are in one or more
//! workspaces, and the App of the instance or of the settings filters them.

use scylla_domain::integrations::apps::VcsAppId;
use scylla_domain::integrations::connections::{ConnectionState, VcsConnectionId};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;

use super::health::Changed;
use crate::Error;

/// The reason of a link whose repository the installation no longer sees.
pub(crate) const REPOSITORY_NOT_VISIBLE: &str = "REPOSITORY_NOT_VISIBLE";

/// A connection whose state changed, with its state before the change.
pub(crate) struct StateChange {
    /// The connection after the change.
    pub(crate) changed: Changed,
    /// The state before the change.
    pub(crate) from: String,
}

/// Moves each connection of an installation in one of the states `from` to a new state with a
/// reason. Returns the connections that changed.
pub(crate) async fn set_state(
    tx: &mut Tx<'_>,
    app: Option<VcsAppId>,
    installation: &str,
    to: (ConnectionState, Option<&str>),
    from: &[ConnectionState],
) -> Result<Vec<StateChange>, Error> {
    let from: Vec<String> = from.iter().map(|state| state.as_str().to_owned()).collect();
    let rows = sqlx::query!(
        r#"UPDATE vcs_connections c
           SET state = $3, state_reason = $4, updated_at = now()
           FROM (SELECT id, state FROM vcs_connections
                 WHERE provider = 'github' AND external_id = $1
                   AND app_id IS NOT DISTINCT FROM $2 AND state = ANY($5)
                 FOR UPDATE) old
           WHERE c.id = old.id
           RETURNING c.id, c.workspace_id, c.provider, c.account, c.token_expires_at,
                     old.state AS "from_state!""#,
        installation,
        app.map(VcsAppId::to_db),
        to.0.as_str(),
        to.1,
        &from[..],
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| StateChange {
            changed: Changed {
                id: VcsConnectionId::from_db(row.id),
                workspace: WorkspaceId::from_db(row.workspace_id),
                provider: row.provider,
                account: row.account,
                expires_at: row.token_expires_at,
            },
            from: row.from_state,
        })
        .collect())
}

/// Keeps the new permissions of each active connection of an installation, or of each one that
/// needs a person, with the state and the reason that they give. Returns the connections whose
/// state changed.
pub(crate) async fn set_permissions(
    tx: &mut Tx<'_>,
    app: Option<VcsAppId>,
    installation: &str,
    permissions: &serde_json::Value,
    to: (ConnectionState, Option<&str>),
) -> Result<Vec<StateChange>, Error> {
    let rows = sqlx::query!(
        r#"UPDATE vcs_connections c
           SET permissions = $3, state = $4, state_reason = $5, updated_at = now()
           FROM (SELECT id, state FROM vcs_connections
                 WHERE provider = 'github' AND external_id = $1
                   AND app_id IS NOT DISTINCT FROM $2
                   AND state IN ('active', 'needs_attention')
                 FOR UPDATE) old
           WHERE c.id = old.id
           RETURNING c.id, c.workspace_id, c.provider, c.account, c.token_expires_at,
                     old.state AS "from_state!""#,
        installation,
        app.map(VcsAppId::to_db),
        permissions,
        to.0.as_str(),
        to.1,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .filter(|row| row.from_state != to.0.as_str())
        .map(|row| StateChange {
            changed: Changed {
                id: VcsConnectionId::from_db(row.id),
                workspace: WorkspaceId::from_db(row.workspace_id),
                provider: row.provider,
                account: row.account,
                expires_at: row.token_expires_at,
            },
            from: row.from_state,
        })
        .collect())
}

/// Keeps the repositories that an installation sees: all of them or a selection, and the count
/// after `added` new ones and `removed` old ones.
pub(crate) async fn set_repositories(
    tx: &mut Tx<'_>,
    app: Option<VcsAppId>,
    installation: &str,
    all: bool,
    (added, removed): (usize, usize),
) -> Result<(), Error> {
    let count = |value: usize| i32::try_from(value).unwrap_or(i32::MAX);
    sqlx::query!(
        "UPDATE vcs_connections
         SET all_repositories = $3,
             repository_count = GREATEST(repository_count + $4 - $5, 0), updated_at = now()
         WHERE provider = 'github' AND external_id = $1 AND app_id IS NOT DISTINCT FROM $2",
        installation,
        app.map(VcsAppId::to_db),
        all,
        count(added),
        count(removed),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Moves the active links of the repositories `removed` of an installation to
/// `needs_attention` with `REPOSITORY_NOT_VISIBLE`, and the links of the repositories `added`
/// with this reason back to `active`. Returns the projects of the links that changed, the
/// stopped ones first.
pub(crate) async fn set_visible(
    tx: &mut Tx<'_>,
    app: Option<VcsAppId>,
    installation: &str,
    added: &[String],
    removed: &[String],
) -> Result<(Vec<ProjectId>, Vec<ProjectId>), Error> {
    let stopped = sqlx::query_scalar!(
        "UPDATE repository_links l
         SET state = 'needs_attention', state_reason = $4, updated_at = now()
         FROM vcs_connections c
         WHERE c.id = l.connection_id AND c.provider = 'github' AND c.external_id = $1
           AND c.app_id IS NOT DISTINCT FROM $2 AND l.external_id = ANY($3)
           AND l.state = 'active'
         RETURNING l.project_id",
        installation,
        app.map(VcsAppId::to_db),
        removed,
        REPOSITORY_NOT_VISIBLE,
    )
    .fetch_all(&mut **tx)
    .await?;
    let restored = sqlx::query_scalar!(
        "UPDATE repository_links l
         SET state = 'active', state_reason = NULL, updated_at = now()
         FROM vcs_connections c
         WHERE c.id = l.connection_id AND c.provider = 'github' AND c.external_id = $1
           AND c.app_id IS NOT DISTINCT FROM $2 AND l.external_id = ANY($3)
           AND l.state = 'needs_attention' AND l.state_reason = $4
         RETURNING l.project_id",
        installation,
        app.map(VcsAppId::to_db),
        added,
        REPOSITORY_NOT_VISIBLE,
    )
    .fetch_all(&mut **tx)
    .await?;
    let ids = |rows: Vec<uuid::Uuid>| rows.into_iter().map(ProjectId::from_db).collect();
    Ok((ids(stopped), ids(restored)))
}
