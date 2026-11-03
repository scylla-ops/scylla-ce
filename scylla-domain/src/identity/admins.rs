//! The instance admins and the console of the users: the routes on the platform.

use crate::identity::actors::ActorId;
use crate::identity::users::User;
use crate::platform::Platform;
use crate::{Page, PageRequest};

/// One page of users, in id order.
pub type UserPage = Page<User, ActorId>;

/// The input of a user list.
#[derive(Clone, Debug, Default)]
pub struct ListUsers {
    /// A text that the email, the handle or the display name of each user contains, in any
    /// case. An empty text keeps each user.
    pub query: String,
    /// The page to return.
    pub page: PageRequest<ActorId>,
}

crate::authz! {
    /// The routes of the console of the users, on the platform. Only the instance admins pass
    /// them: no role gives them.
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
