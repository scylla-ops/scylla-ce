//! The user's writes. Each command, its access and its payload types. The strata module of the
//! slice runs it.

use super::UserSession;
use crate::domain::ids::{SessionId, UserId};
use crate::domain::permission::Permission;
use crate::domain::user::{DisplayName, Email, Password, User, Username};
use scylla_extension::{Access, Command, Deleted, Describe, Draft};

#[derive(Debug)]
pub struct CreateUser {
    pub username: Username,
    pub email: Email,
    pub password: Password,
    pub display_name: Option<DisplayName>,
}

impl Describe for CreateUser {
    fn access(&self) -> Access {
        Access::Requires(Permission::CreateUser)
    }
}

impl Command for CreateUser {
    type Staged = Draft<User>;
    type Committed = User;
}

/// `None` leaves a field as it is; `display_name: Some(None)` removes the display name.
#[derive(Debug)]
pub struct UpdateUser {
    pub id: UserId,
    pub username: Option<Username>,
    pub display_name: Option<Option<DisplayName>>,
    pub email: Option<Email>,
}

/// An email is the address of the reset links, so setting one also needs `createUser`, also on
/// the caller's own account.
impl Describe for UpdateUser {
    fn access(&self) -> Access {
        let update = Permission::UpdateUser(self.id.clone());
        if self.email.is_some() {
            Access::RequiresAll(vec![update, Permission::CreateUser])
        } else {
            Access::Requires(update)
        }
    }
}

impl Command for UpdateUser {
    type Staged = Draft<User>;
    type Committed = User;
}

/// No RPC sends it; the bootstrap gives its email to an admin account that has none. An account
/// with another email keeps it.
#[derive(Debug)]
pub struct UpdateUserEmail {
    pub id: UserId,
    pub email: Email,
}

impl Describe for UpdateUserEmail {
    fn access(&self) -> Access {
        Access::Requires(Permission::UpdateUser(self.id.clone()))
    }
}

impl Command for UpdateUserEmail {
    type Staged = Draft<User>;
    type Committed = User;
}

#[derive(Debug)]
pub struct DeleteUser {
    pub id: UserId,
}

impl Describe for DeleteUser {
    fn access(&self) -> Access {
        Access::Requires(Permission::DeleteUser(self.id.clone()))
    }
}

impl Command for DeleteUser {
    type Staged = User;
    type Committed = Deleted<User>;
}

/// `session` is the session of the call, which stays open.
#[derive(Debug)]
pub struct ChangePassword {
    pub current_password: Password,
    pub new_password: Password,
    pub session: Option<SessionId>,
}

impl Describe for ChangePassword {
    fn access(&self) -> Access {
        Access::Authenticated
    }
}

impl Command for ChangePassword {
    type Staged = Draft<User>;
    type Committed = User;
}

/// `session` is the session of the call. It stays open only when the user is the caller.
#[derive(Debug)]
pub struct RevokeUserSessions {
    pub id: UserId,
    pub session: Option<SessionId>,
}

impl Describe for RevokeUserSessions {
    fn access(&self) -> Access {
        Access::Requires(Permission::UpdateUser(self.id.clone()))
    }
}

impl Command for RevokeUserSessions {
    type Staged = Option<SessionId>;
    type Committed = u64;
}

/// Revokes one session of the user. The session of the call can be revoked: the caller is then
/// signed out. An unknown id, a session of another user and an expired session give the same
/// `NotFound`.
#[derive(Debug)]
pub struct RevokeUserSession {
    pub id: UserId,
    pub session_id: SessionId,
}

impl Describe for RevokeUserSession {
    fn access(&self) -> Access {
        Access::Requires(Permission::UpdateUser(self.id.clone()))
    }
}

impl Command for RevokeUserSession {
    type Staged = UserSession;
    type Committed = Deleted<UserSession>;
}

/// The caller's own account gets no exemption: the caller cannot change its own flag.
#[derive(Debug)]
pub struct SetUserActive {
    pub id: UserId,
    pub is_active: bool,
}

impl Describe for SetUserActive {
    fn access(&self) -> Access {
        Access::Requires(Permission::UpdateUser(self.id.clone()))
    }
}

/// The flag tells whether the value changed: setting the current value writes nothing.
impl Command for SetUserActive {
    type Staged = (Draft<User>, bool);
    type Committed = User;
}

/// The deletion of the caller's own account, with the effects of `DeleteUser`.
#[derive(Debug)]
pub struct DeleteAccount {
    pub password: Password,
}

impl Describe for DeleteAccount {
    fn access(&self) -> Access {
        Access::Authenticated
    }
}

impl Command for DeleteAccount {
    type Staged = User;
    type Committed = Deleted<User>;
}
