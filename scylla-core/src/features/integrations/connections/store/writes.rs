//! The writes of the `vcs_connections` table: an installation of the GitHub App, and the sealed
//! token of a GitLab service account. Each function runs in the transaction of the caller. A
//! token is sealed before it comes here.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::integrations::apps::VcsAppId;
use scylla_domain::integrations::connections::{AccountKind, ConnectionState, VcsConnectionId};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{Rejection, Tx};
use time::OffsetDateTime;

use crate::Error;

/// The unique constraint of an account in a workspace.
const ACCOUNT_KEY: &str = "vcs_connections_workspace_id_provider_external_id_key";

/// The foreign key from a connection to its workspace.
const WORKSPACE_FOREIGN_KEY: &str = "vcs_connections_workspace_id_fkey";

/// The values of an installation of the GitHub App.
pub(crate) struct Installation<'a> {
    /// The workspace.
    pub(crate) workspace: WorkspaceId,
    /// The App of the instance, or `None` for the App of the settings.
    pub(crate) app: Option<VcsAppId>,
    /// The id of the installation.
    pub(crate) external_id: &'a str,
    /// The login of the account.
    pub(crate) account: &'a str,
    /// The kind of the account.
    pub(crate) account_kind: AccountKind,
    /// The avatar of the account.
    pub(crate) avatar_url: Option<&'a str>,
    /// The permissions, as a JSON object.
    pub(crate) permissions: &'a serde_json::Value,
    /// `true` for all the repositories of the account.
    pub(crate) all_repositories: bool,
    /// The number of repositories.
    pub(crate) repository_count: u32,
    /// The state.
    pub(crate) state: ConnectionState,
    /// The reason of a state that is not active.
    pub(crate) state_reason: Option<&'a str>,
    /// The actor that makes the connection.
    pub(crate) created_by: ActorId,
}

/// The values of the token of a GitLab service account.
pub(crate) struct Token<'a> {
    /// The id of the user on GitLab.
    pub(crate) external_id: &'a str,
    /// The login of the account.
    pub(crate) account: &'a str,
    /// The kind of the account.
    pub(crate) account_kind: AccountKind,
    /// The avatar of the account.
    pub(crate) avatar_url: Option<&'a str>,
    /// The token, sealed with the id of the connection.
    pub(crate) sealed: &'a [u8],
    /// The expiry of the token.
    pub(crate) expires_at: Option<OffsetDateTime>,
    /// The scopes of the token.
    pub(crate) scopes: &'a [String],
    /// The number of projects that the account sees.
    pub(crate) repository_count: u32,
}

/// Makes the connection of an installation in a workspace, or refreshes it when the workspace
/// has it. Returns the id of the connection, and `true` for a new connection.
pub(crate) async fn upsert_installation(
    tx: &mut Tx<'_>,
    id: VcsConnectionId,
    values: &Installation<'_>,
) -> Result<(VcsConnectionId, bool), Error> {
    let kept = sqlx::query_scalar!(
        "INSERT INTO vcs_connections (id, workspace_id, provider, auth_kind, app_id, external_id,
                                      account, account_kind, avatar_url, permissions,
                                      all_repositories, repository_count, state, state_reason,
                                      created_by)
         VALUES ($1, $2, 'github', 'app_installation', $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                 $13)
         ON CONFLICT (workspace_id, provider, external_id) DO UPDATE SET
             app_id = EXCLUDED.app_id, account = EXCLUDED.account,
             account_kind = EXCLUDED.account_kind, avatar_url = EXCLUDED.avatar_url,
             permissions = EXCLUDED.permissions, all_repositories = EXCLUDED.all_repositories,
             repository_count = EXCLUDED.repository_count, state = EXCLUDED.state,
             state_reason = EXCLUDED.state_reason, updated_at = now()
         RETURNING id",
        id.to_db(),
        values.workspace.to_db(),
        values.app.map(VcsAppId::to_db),
        values.external_id,
        values.account,
        values.account_kind.as_str(),
        values.avatar_url,
        values.permissions,
        values.all_repositories,
        count(values.repository_count),
        values.state.as_str(),
        values.state_reason,
        values.created_by.to_db(),
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(workspace_error)?;
    let kept = VcsConnectionId::from_db(kept);
    Ok((kept, kept == id))
}

/// Inserts the connection of a GitLab token. An account with a connection in the workspace gives
/// `CONNECTION_EXISTS`.
pub(crate) async fn insert_token(
    tx: &mut Tx<'_>,
    id: VcsConnectionId,
    workspace: WorkspaceId,
    created_by: ActorId,
    token: &Token<'_>,
) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO vcs_connections (id, workspace_id, provider, auth_kind, external_id, account,
                                      account_kind, avatar_url, sealed_token, token_expires_at,
                                      scopes, repository_count, created_by)
         VALUES ($1, $2, 'gitlab', 'access_token', $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        id.to_db(),
        workspace.to_db(),
        token.external_id,
        token.account,
        token.account_kind.as_str(),
        token.avatar_url,
        token.sealed,
        token.expires_at,
        token.scopes,
        count(token.repository_count),
        created_by.to_db(),
    )
    .execute(&mut **tx)
    .await
    .map_err(workspace_error)?;
    Ok(())
}

/// Replaces the token of a GitLab connection, and makes the connection active.
pub(crate) async fn replace_token(
    tx: &mut Tx<'_>,
    id: VcsConnectionId,
    token: &Token<'_>,
) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE vcs_connections
         SET account = $2, account_kind = $3, avatar_url = $4, sealed_token = $5,
             token_expires_at = $6, token_warned_at = NULL, scopes = $7, repository_count = $8,
             state = 'active', state_reason = NULL, updated_at = now()
         WHERE id = $1",
        id.to_db(),
        token.account,
        token.account_kind.as_str(),
        token.avatar_url,
        token.sealed,
        token.expires_at,
        token.scopes,
        count(token.repository_count),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns a count as a column value.
fn count(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// Maps the errors of a write: an account in use and an unknown workspace.
fn workspace_error(error: sqlx::Error) -> Error {
    match error {
        sqlx::Error::Database(db) if db.constraint() == Some(ACCOUNT_KEY) => {
            Error::Rejected(Rejection::new(
                "CONNECTION_EXISTS",
                "the workspace has a connection of this account: replace its token",
            ))
        }
        sqlx::Error::Database(db) if db.constraint() == Some(WORKSPACE_FOREIGN_KEY) => {
            Error::NotFound("workspace")
        }
        error => error.into(),
    }
}
