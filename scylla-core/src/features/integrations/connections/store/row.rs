//! A row of the `vcs_connections` table with the number of its projects, and the connection that
//! it gives. No column of the row holds a token.

use std::collections::BTreeMap;

use scylla_domain::identity::actors::ActorId;
use scylla_domain::integrations::apps::VcsAppId;
use scylla_domain::integrations::connections::{
    AccountKind, AuthKind, ConnectionState, DeliveryMode, VcsConnection, VcsConnectionId,
};
use scylla_domain::integrations::VcsProvider;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::Error;

/// A connection and the number of projects that link a repository of it.
#[derive(Clone, Debug)]
pub(crate) struct Counted {
    /// The connection.
    pub(crate) connection: VcsConnection,
    /// The number of projects with a repository of the connection.
    pub(crate) projects: u32,
}

/// A row of `vcs_connections`, with the count of its links.
pub(super) struct Row {
    /// The id.
    pub(super) id: Uuid,
    /// The workspace.
    pub(super) workspace_id: Uuid,
    /// The provider.
    pub(super) provider: String,
    /// The auth kind.
    pub(super) auth_kind: String,
    /// The App of the instance.
    pub(super) app_id: Option<Uuid>,
    /// The id on the provider.
    pub(super) external_id: String,
    /// The login of the account.
    pub(super) account: String,
    /// The kind of the account.
    pub(super) account_kind: String,
    /// The avatar of the account.
    pub(super) avatar_url: Option<String>,
    /// The expiry of a token.
    pub(super) token_expires_at: Option<OffsetDateTime>,
    /// The scopes of a token.
    pub(super) scopes: Vec<String>,
    /// The permissions of an installation.
    pub(super) permissions: serde_json::Value,
    /// `true` for all the repositories of the account.
    pub(super) all_repositories: bool,
    /// The number of repositories.
    pub(super) repository_count: i32,
    /// The delivery mode.
    pub(super) delivery_mode: String,
    /// The time between two polls.
    pub(super) poll_interval_seconds: i32,
    /// The time of the last webhook.
    pub(super) last_event_at: Option<OffsetDateTime>,
    /// The state.
    pub(super) state: String,
    /// The reason of the state.
    pub(super) state_reason: Option<String>,
    /// The author.
    pub(super) created_by: Uuid,
    /// The creation time.
    pub(super) created_at: OffsetDateTime,
    /// The number of links.
    pub(super) project_count: i64,
}

impl Row {
    /// Makes the connection of the row. The table checks each value, so a bad value is an
    /// internal error.
    pub(super) fn into_counted(self) -> Result<Counted, Error> {
        let permissions: BTreeMap<String, String> =
            serde_json::from_value(self.permissions).map_err(|_| bad("permissions", &self.id))?;
        let connection = VcsConnection {
            id: VcsConnectionId::from_db(self.id),
            workspace_id: WorkspaceId::from_db(self.workspace_id),
            provider: VcsProvider::parse(&self.provider)
                .ok_or_else(|| bad("provider", &self.id))?,
            auth_kind: AuthKind::parse(&self.auth_kind).ok_or_else(|| bad("auth", &self.id))?,
            app_id: self.app_id.map(VcsAppId::from_db),
            external_id: self.external_id,
            account: self.account,
            account_kind: AccountKind::parse(&self.account_kind)
                .ok_or_else(|| bad("account kind", &self.id))?,
            avatar_url: self.avatar_url,
            token_expires_at: self.token_expires_at,
            scopes: self.scopes,
            permissions,
            all_repositories: self.all_repositories,
            repository_count: u32::try_from(self.repository_count).unwrap_or_default(),
            delivery_mode: DeliveryMode::parse(&self.delivery_mode)
                .ok_or_else(|| bad("delivery mode", &self.id))?,
            poll_interval_seconds: u32::try_from(self.poll_interval_seconds).unwrap_or_default(),
            last_event_at: self.last_event_at,
            state: ConnectionState::parse(&self.state).ok_or_else(|| bad("state", &self.id))?,
            state_reason: self.state_reason,
            created_by: ActorId::from_db(self.created_by),
            created_at: self.created_at,
        };
        Ok(Counted {
            connection,
            projects: u32::try_from(self.project_count).unwrap_or_default(),
        })
    }
}

/// The error of a stored value that is not valid.
fn bad(what: &str, id: &Uuid) -> Error {
    Error::Internal(format!("the {what} of the connection {id} is not valid"))
}
