//! The instance admins and the console of the users: the routes on the platform.

use crate::identity::actors::ActorId;
use time::OffsetDateTime;

use crate::identity::users::User;
use crate::platform::images::Icon;
use crate::platform::Platform;
use crate::tenancy::workspaces::WorkspaceId;
use crate::{Page, PageRequest};

/// One page of users, in id order.
pub type UserPage = Page<User, ActorId>;

/// The input of a user list.
#[derive(Clone, Debug, Default)]
pub struct ListUsers {
    /// A text that the email, the handle or the display name of each user contains, in any
    /// case. An empty text keeps each user.
    pub query: String,
    /// `true` keeps only the users with an instance role.
    pub with_instance_role: bool,
    /// The page to return.
    pub page: PageRequest<ActorId>,
}

/// An instance role of a user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceRole {
    /// The key of the role.
    pub key: String,
    /// The name of the role.
    pub name: String,
    /// `true` if the role comes from the server.
    pub builtin: bool,
}

/// One user of the console, with its place on the instance.
#[derive(Clone, Debug)]
pub struct UserEntry {
    /// The user.
    pub user: User,
    /// The roles of the user on the whole instance.
    pub instance_roles: Vec<InstanceRole>,
    /// The workspaces that the user owns or has a role in, in name order.
    pub workspaces: Vec<UserWorkspace>,
    /// The time of the last sign-in.
    pub last_sign_in_at: Option<OffsetDateTime>,
}

/// A workspace that a user of the console owns or has a role in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserWorkspace {
    /// The workspace.
    pub id: WorkspaceId,
    /// The name of the workspace.
    pub name: String,
    /// The custom icon of the workspace, if it has one.
    pub icon: Option<Icon>,
    /// `true` if the user is an owner of the workspace.
    pub owner: bool,
    /// The display name of the first owner, if the user is not an owner.
    pub owner_name: Option<String>,
}

/// One page of the console of the users.
#[derive(Clone, Debug)]
pub struct UserEntries {
    /// The users of the page.
    pub page: Page<UserEntry, ActorId>,
    /// The number of users that the query keeps.
    pub total: u32,
    /// The number of users that the query keeps, with an instance role.
    pub total_with_instance_role: u32,
}

crate::authz! {
    /// The routes of the console of the users, on the platform. The instance admins pass them,
    /// and a role of the platform level gives them through the catalog.
    extend Platform as UserAdminHolder {
        /// The route `ListUsers`.
        permission list_users = nil;
        /// The route `GetUser`.
        permission get_user = nil;
        /// The route `CreateUser`.
        permission create_user = nil;
        /// The route `UpdateUser`.
        permission update_user = nil;
        /// The route `SetUserStatus`.
        permission set_user_status = nil;
        /// The route `DeleteUser`.
        permission delete_user = nil;
        /// The route `SendPasswordReset`.
        permission send_password_reset = nil;
        /// The route `ListUserSessions`.
        permission list_user_sessions = nil;
        /// The route `RevokeUserSessions`.
        permission revoke_user_sessions = nil;
        /// The route `GrantInstanceAdmin`.
        permission grant_instance_admin = nil;
        /// The route `RevokeInstanceAdmin`.
        permission revoke_instance_admin = nil;
    }
}
