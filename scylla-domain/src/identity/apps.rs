//! The Apps: the machine identities of a workspace. An agent is an App that runs jobs.

use time::OffsetDateTime;

use crate::identity::actors::{Actor, ActorId, ActorKind};
use crate::identity::agents::AgentLabels;
pub use crate::identity::agents::Host;
use crate::tenancy::workspaces::{Workspace, WorkspaceId};
use crate::{Id, Name, Page, PageRequest};

/// The actor kind of an App.
pub const KIND: ActorKind = ActorKind::new("app");

/// The id of an App. Its actor has the same id: `App::actor`.
pub type AppId = Id<App>;

/// The display name of an App.
pub type AppName = Name;

/// The name of a key of an App, for example the host name of its machine.
pub type KeyLabel = Name;

/// The marker of an App key id.
pub enum AppKey {}

/// The id of a public key of an App.
pub type AppKeyId = Id<AppKey>;

/// An App, as the store returns it.
#[derive(Clone, Debug)]
pub struct App {
    /// The id of the App and of its actor.
    pub id: AppId,
    /// The workspace of the App.
    pub workspace_id: WorkspaceId,
    /// The display name.
    pub name: String,
    /// `true` for an agent: an App that runs jobs.
    pub runner: bool,
    /// The actor that created the App.
    pub created_by: ActorId,
    /// The creation time.
    pub created_at: OffsetDateTime,
    /// The last open, renewal or close of a stream of the agent.
    pub last_seen_at: Option<OffsetDateTime>,
    /// The host of the last stream, or `None` before the first stream.
    pub host: Option<Host>,
    /// The time when an admin disabled the App, or `None` while it is enabled.
    pub disabled_at: Option<OffsetDateTime>,
    /// The labels that the admins gave to the App.
    pub labels: AgentLabels,
}

impl App {
    /// Returns the id of the actor of an App.
    pub fn actor(id: AppId) -> ActorId {
        id.retype()
    }

    /// Returns the App id of an actor of kind `app`.
    pub fn of_actor(id: ActorId) -> AppId {
        id.retype()
    }
}

/// A public key of an App, with no key bytes when it comes from a list.
#[derive(Clone, Debug)]
pub struct AppKeyInfo {
    /// The id of the key.
    pub id: AppKeyId,
    /// The Ed25519 public key, 32 bytes.
    pub public_key: Vec<u8>,
    /// The name of the key, if it has one.
    pub label: Option<String>,
    /// The time when the key was added.
    pub created_at: OffsetDateTime,
    /// The time when the key was revoked.
    pub revoked_at: Option<OffsetDateTime>,
    /// The last time that the key signed an assertion for a token.
    pub last_used_at: Option<OffsetDateTime>,
}

/// The input of an App creation.
#[derive(Clone, Debug)]
pub struct NewApp {
    /// The workspace of the App.
    pub workspace_id: WorkspaceId,
    /// The actor that creates the App.
    pub created_by: ActorId,
    /// The display name.
    pub name: AppName,
    /// `true` for an agent.
    pub runner: bool,
}

/// The input of an App list.
#[derive(Clone, Copy, Debug)]
pub struct ListApps {
    /// The workspace.
    pub workspace_id: WorkspaceId,
    /// Only the agents, only the other Apps, or all if `None`.
    pub runner: Option<bool>,
    /// The page to return.
    pub page: PageRequest<AppId>,
}

/// One page of Apps.
pub type AppPage = Page<App, AppId>;

/// A join code of an App, and its expiry.
#[derive(Clone, Debug)]
pub struct JoinCode {
    /// The code, in clear. Only its hash is in the database.
    pub code: String,
    /// The end of its validity.
    pub expires_at: OffsetDateTime,
}

crate::authz! {
    /// The routes on one App: its creator, and the roles of its workspace, which is in Postgres.
    definition app for App {
        /// The actor that created the App: it manages the App, also with no role in the
        /// workspace.
        relation creator: Actor;
        /// The route `GetApp`.
        permission get_app = creator;
        /// The route `DeleteApp`.
        permission delete_app = creator;
        /// The route `DisableApp`.
        permission disable_app = creator;
        /// The route `EnableApp`.
        permission enable_app = creator;
        /// The route `CreateJoinCode`.
        permission create_join_code = creator;
        /// The route `AddAppKey`.
        permission add_app_key = creator;
        /// The route `RevokeAppKey`.
        permission revoke_app_key = creator;
        /// The route `RenameAppKey`.
        permission rename_app_key = creator;
        /// The route `GetAgentStats`.
        permission get_agent_stats = creator;
        /// The route `CreateAppToken`.
        permission create_app_token = creator;
        /// The route `ListAppTokens`.
        permission list_app_tokens = creator;
        /// The route `RevokeAppToken`.
        permission revoke_app_token = creator;
        /// The route `AddCiIdentity`.
        permission add_ci_identity = creator;
        /// The route `ListCiIdentities`.
        permission list_ci_identities = creator;
        /// The route `RemoveCiIdentity`.
        permission remove_ci_identity = creator;
        /// The route `SetAppLabels`: the admins of the workspace only, not the creator, so the
        /// machine of an agent cannot choose its jobs.
        permission set_app_labels = nil;
    }

    /// The routes of the Apps on their workspace.
    extend Workspace as AppHolder {
        /// The role that creates Apps in this workspace, with no other right in it.
        relation app_creator: Actor;
        /// The route `CreateApp`, in this workspace: its admins and its App creators.
        permission create_app = manage + app_creator;
        /// The route `GetApp`, on each App of this workspace.
        permission get_app = read;
        /// The route `ListApps`: the Apps of this workspace.
        permission list_apps = read;
        /// The route `DeleteApp`, on each App of this workspace.
        permission delete_app = manage;
        /// The route `DisableApp`, on each App of this workspace.
        permission disable_app = manage;
        /// The route `EnableApp`, on each App of this workspace.
        permission enable_app = manage;
        /// The route `CreateJoinCode`, on each App of this workspace.
        permission create_join_code = manage;
        /// The route `AddAppKey`, on each App of this workspace.
        permission add_app_key = manage;
        /// The route `RevokeAppKey`, on each App of this workspace.
        permission revoke_app_key = manage;
        /// The route `RenameAppKey`, on each App of this workspace.
        permission rename_app_key = manage;
        /// The route `GetAgentStats`, on each App of this workspace.
        permission get_agent_stats = read;
        /// The route `ListAgentStats`: the agents of this workspace.
        permission list_agent_stats = read;
        /// The route `CreateAppToken`, on each App of this workspace.
        permission create_app_token = manage;
        /// The route `ListAppTokens`, on each App of this workspace.
        permission list_app_tokens = read;
        /// The route `RevokeAppToken`, on each App of this workspace.
        permission revoke_app_token = manage;
        /// The route `AddCiIdentity`, on each App of this workspace.
        permission add_ci_identity = manage;
        /// The route `ListCiIdentities`, on each App of this workspace.
        permission list_ci_identities = read;
        /// The route `RemoveCiIdentity`, on each App of this workspace.
        permission remove_ci_identity = manage;
        /// The route `SetAppLabels`, on each App of this workspace.
        permission set_app_labels = manage;
    }
}
