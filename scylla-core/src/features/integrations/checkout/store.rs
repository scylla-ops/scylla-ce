//! The SQL of the checkout: the repository of a project with its connection, and the rows of the
//! GitLab deploy tokens. No row holds a token.

use scylla_domain::integrations::apps::VcsAppId;
use scylla_domain::integrations::checkout::{CheckoutToken, CheckoutTokenId};
use scylla_domain::integrations::connections::{ConnectionState, VcsConnectionId};
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::jobs::JobId;
use scylla_domain::projects::ProjectId;
use scylla_extension::Tx;

use crate::Error;

/// The repository of a project, with the connection that sees it.
pub(super) struct Target {
    /// The provider.
    pub(super) provider: VcsProvider,
    /// The connection.
    pub(super) connection: VcsConnectionId,
    /// The App of the instance of the connection, or `None` for the App of the settings and for
    /// a token.
    pub(super) app: Option<VcsAppId>,
    /// The id of the connection on the provider: the installation on GitHub.
    pub(super) account_id: String,
    /// The id of the repository on the provider.
    pub(super) repository_id: String,
    /// The full name of the repository, for example `acme/api`.
    pub(super) full_name: String,
    /// The page of the repository on the provider.
    pub(super) web_url: String,
    /// The state of the link, or the state of the connection when it is not active.
    pub(super) state: ConnectionState,
}

/// Returns the repository of a project and its connection, or `None` for a project with no link.
pub(super) async fn target(tx: &mut Tx<'_>, project: ProjectId) -> Result<Option<Target>, Error> {
    let row = sqlx::query!(
        "SELECT c.id AS connection_id, c.provider, c.app_id, c.external_id AS account_id,
                c.state AS connection_state, l.external_id, l.full_name, l.web_url, l.state
         FROM repository_links l JOIN vcs_connections c ON c.id = l.connection_id
         WHERE l.project_id = $1",
        project.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let unknown = |what: &str| Error::Internal(format!("unknown {what} of a repository link"));
    let link = ConnectionState::parse(&row.state).ok_or_else(|| unknown("state"))?;
    let connection =
        ConnectionState::parse(&row.connection_state).ok_or_else(|| unknown("state"))?;
    Ok(Some(Target {
        provider: VcsProvider::parse(&row.provider).ok_or_else(|| unknown("provider"))?,
        connection: VcsConnectionId::from_db(row.connection_id),
        app: row.app_id.map(VcsAppId::from_db),
        account_id: row.account_id,
        repository_id: row.external_id,
        full_name: row.full_name,
        web_url: row.web_url,
        state: match connection {
            ConnectionState::Active => link,
            _ => connection,
        },
    }))
}

/// Inserts the row of a deploy token.
pub(super) async fn insert(tx: &mut Tx<'_>, token: &CheckoutToken) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO vcs_checkout_tokens (id, job_id, connection_id, project_external_id,
                                          token_external_id, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
        token.id.to_db(),
        token.job_id.map(JobId::to_db),
        token.connection_id.to_db(),
        token.project_external_id,
        token.token_external_id,
        token.expires_at,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns at most `limit` deploy tokens that no job needs, the oldest expiry first: the token of
/// a job that ended, that no agent holds or that was deleted, and a token that expired.
pub(super) async fn unused(tx: &mut Tx<'_>, limit: i64) -> Result<Vec<CheckoutToken>, Error> {
    let rows = sqlx::query!(
        "SELECT t.id, t.job_id, t.connection_id, t.project_external_id, t.token_external_id,
                t.expires_at
         FROM vcs_checkout_tokens t LEFT JOIN jobs j ON j.id = t.job_id
         WHERE j.id IS NULL OR j.agent_id IS NULL OR t.expires_at <= now()
            OR j.status NOT IN ('awaiting_approval', 'pending', 'running')
         ORDER BY t.expires_at
         LIMIT $1",
        limit,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| CheckoutToken {
            id: CheckoutTokenId::from_db(row.id),
            job_id: row.job_id.map(JobId::from_db),
            connection_id: VcsConnectionId::from_db(row.connection_id),
            project_external_id: row.project_external_id,
            token_external_id: row.token_external_id,
            expires_at: row.expires_at,
        })
        .collect())
}

/// Deletes the row of a deploy token.
pub(super) async fn delete(tx: &mut Tx<'_>, id: CheckoutTokenId) -> Result<(), Error> {
    sqlx::query!("DELETE FROM vcs_checkout_tokens WHERE id = $1", id.to_db())
        .execute(&mut **tx)
        .await?;
    Ok(())
}
