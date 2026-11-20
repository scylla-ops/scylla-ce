//! The SQL of the commit context: the repository of a project and the connection that sees it.

use scylla_domain::integrations::apps::VcsAppId;
use scylla_domain::integrations::connections::{ConnectionState, VcsConnectionId};
use scylla_domain::integrations::VcsProvider;
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
    /// The token of a GitLab service account, sealed with the id of the connection.
    pub(super) sealed_token: Option<Vec<u8>>,
    /// The id of the repository on the provider.
    pub(super) repository_id: String,
    /// The full name of the repository, for example `acme/api`.
    pub(super) full_name: String,
    /// The page of the repository on the provider.
    pub(super) web_url: String,
    /// The state of the link, or the state of the connection when it is not active.
    pub(super) state: ConnectionState,
    /// The reason of a state that is not active.
    pub(super) reason: Option<String>,
}

/// Returns the repository of a project and its connection, or `None` for a project with no link.
pub(super) async fn target(tx: &mut Tx<'_>, project: ProjectId) -> Result<Option<Target>, Error> {
    let row = sqlx::query!(
        "SELECT c.id AS connection_id, c.provider, c.app_id, c.external_id AS account_id,
                c.sealed_token, c.state AS connection_state, c.state_reason AS connection_reason,
                l.external_id, l.full_name, l.web_url, l.state, l.state_reason
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
    let (state, reason) = match connection {
        ConnectionState::Active => (link, row.state_reason),
        _ => (connection, row.connection_reason),
    };
    Ok(Some(Target {
        provider: VcsProvider::parse(&row.provider).ok_or_else(|| unknown("provider"))?,
        connection: VcsConnectionId::from_db(row.connection_id),
        app: row.app_id.map(VcsAppId::from_db),
        account_id: row.account_id,
        sealed_token: row.sealed_token,
        repository_id: row.external_id,
        full_name: row.full_name,
        web_url: row.web_url,
        state,
        reason,
    }))
}
