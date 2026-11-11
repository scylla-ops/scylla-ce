//! The connections of a workspace to a Git provider: an installation of the GitHub App, or the
//! token of a GitLab service account. All the projects of the workspace can use a connection.

use std::collections::BTreeMap;

use time::OffsetDateTime;

use super::apps::VcsAppId;
use super::{named, VcsProvider};
use crate::identity::actors::ActorId;
use crate::tenancy::workspaces::{Workspace, WorkspaceId};
use crate::Id;

/// The id of a connection.
pub type VcsConnectionId = Id<VcsConnection>;

named! {
    /// The state of a connection, or of the link of a project.
    pub enum ConnectionState {
        /// Scylla can use the connection.
        Active = "active",
        /// An owner suspended the App on GitHub. Nothing is sent until the owner stops it.
        Suspended = "suspended",
        /// A person must act: the App was uninstalled, a token expired, or a permission is missing.
        NeedsAttention = "needs_attention",
        /// The connection was removed on the provider.
        Disconnected = "disconnected",
    }
}

named! {
    /// How Scylla proves its identity to the provider.
    pub enum AuthKind {
        /// An installation of the GitHub App: Scylla gets short tokens of the installation.
        AppInstallation = "app_installation",
        /// The access token of a service account or of a bot user, sealed in the row.
        AccessToken = "access_token",
    }
}

named! {
    /// The kind of the account of a connection.
    pub enum AccountKind {
        /// A GitHub organization.
        Organization = "organization",
        /// The account of a person.
        User = "user",
        /// A GitLab service account, or the bot user of a group or project access token.
        ServiceAccount = "service_account",
    }
}

named! {
    /// How the provider tells Scylla about the new commits.
    pub enum DeliveryMode {
        /// The provider sends webhooks, and Scylla still polls each 15 minutes.
        Webhook = "webhook",
        /// No webhook comes: Scylla reads the heads at an interval.
        Polling = "polling",
    }
}

/// A connection, as the store returns it. No field holds a token.
#[derive(Clone, Debug)]
pub struct VcsConnection {
    /// The id.
    pub id: VcsConnectionId,
    /// The workspace.
    pub workspace_id: WorkspaceId,
    /// The provider.
    pub provider: VcsProvider,
    /// How Scylla proves its identity.
    pub auth_kind: AuthKind,
    /// The App of the instance, or `None` for the App of the settings and for a token.
    pub app_id: Option<VcsAppId>,
    /// The id on the provider: the installation on GitHub, the user on GitLab.
    pub external_id: String,
    /// The login of the account.
    pub account: String,
    /// The kind of the account.
    pub account_kind: AccountKind,
    /// The address of the avatar of the account.
    pub avatar_url: Option<String>,
    /// The expiry of a token, if it has one.
    pub token_expires_at: Option<OffsetDateTime>,
    /// The scopes of a token.
    pub scopes: Vec<String>,
    /// The permissions of an installation of the GitHub App, by name: `checks` gives `write`.
    pub permissions: BTreeMap<String, String>,
    /// `true` when the connection sees all the repositories of the account.
    pub all_repositories: bool,
    /// The number of repositories that the connection sees.
    pub repository_count: u32,
    /// How the provider tells Scylla about the new commits.
    pub delivery_mode: DeliveryMode,
    /// The time between two polls in polling mode, in seconds.
    pub poll_interval_seconds: u32,
    /// The time of the last webhook of the connection.
    pub last_event_at: Option<OffsetDateTime>,
    /// The state.
    pub state: ConnectionState,
    /// The reason of a state that is not active, as a stable code.
    pub state_reason: Option<String>,
    /// The actor that made the connection.
    pub created_by: ActorId,
    /// The creation time.
    pub created_at: OffsetDateTime,
}

crate::authz! {
    /// The routes of the connections of a workspace to GitHub and GitLab.
    extend Workspace as VcsConnectionHolder {
        /// The route `GetVcsProviders`, in this workspace.
        permission get_vcs_providers = read;
        /// The route `ListVcsConnections`: the connections of this workspace.
        permission list_vcs_connections = read;
        /// The route `GetVcsConnection`, on each connection of this workspace.
        permission get_vcs_connection = read;
        /// The route `StartGitHubInstall`, in this workspace.
        permission start_git_hub_install = manage;
        /// The route `CompleteGitHubInstall`, in this workspace.
        permission complete_git_hub_install = manage;
        /// The route `ConnectGitLab`, in this workspace.
        permission connect_git_lab = manage;
        /// The route `ReplaceGitLabToken`, on each connection of this workspace.
        permission replace_git_lab_token = manage;
        /// The route `DeleteVcsConnection`, on each connection of this workspace.
        permission delete_vcs_connection = manage;
    }
}
