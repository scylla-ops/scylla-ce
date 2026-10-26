//! The user's reads. Each query, its access and its output type. The strata module of the slice
//! runs it.

use super::UserSession;
use crate::application::grant::PrincipalAccess;
use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::domain::ids::{SessionId, UserId};
use crate::domain::permission::Permission;
use crate::domain::user::{Email, User, Username};
use scylla_extension::{Access, Describe, Query};

#[derive(Debug)]
pub struct GetUser {
    pub id: UserId,
}

impl Describe for GetUser {
    fn access(&self) -> Access {
        Access::Requires(Permission::ReadUser(self.id.clone()))
    }
}

impl Query for GetUser {
    type Output = User;
}

/// The account of the caller. It needs no permission; an app or a service has no account.
#[derive(Debug)]
pub struct GetMe;

impl Describe for GetMe {
    fn access(&self) -> Access {
        Access::Authenticated
    }
}

impl Query for GetMe {
    type Output = User;
}

/// No RPC sends it; the bootstrap reads back an admin that already exists.
#[derive(Debug)]
pub struct GetUserByUsername {
    pub username: Username,
}

impl Describe for GetUserByUsername {
    fn access(&self) -> Access {
        Access::Requires(Permission::ListUsers)
    }
}

impl Query for GetUserByUsername {
    type Output = User;
}

/// No RPC sends it; the bootstrap finds the admin account by its configured email.
#[derive(Debug)]
pub struct GetUserByEmail {
    pub email: Email,
}

impl Describe for GetUserByEmail {
    fn access(&self) -> Access {
        Access::Requires(Permission::ListUsers)
    }
}

impl Query for GetUserByEmail {
    type Output = User;
}

#[derive(Debug)]
pub struct ListUsers {
    pub pagination: Option<PaginationParams>,
}

impl Describe for ListUsers {
    fn access(&self) -> Access {
        Access::Requires(Permission::ListUsers)
    }
}

impl Query for ListUsers {
    type Output = PaginatedResult<User>;
}

#[derive(Debug)]
pub struct ListUserAccess {
    pub id: UserId,
}

impl Describe for ListUserAccess {
    fn access(&self) -> Access {
        Access::Requires(Permission::ReadUser(self.id.clone()))
    }
}

impl Query for ListUserAccess {
    type Output = Vec<PrincipalAccess>;
}

/// `session` is the session of the call: the output marks it `current`.
#[derive(Debug)]
pub struct ListUserSessions {
    pub id: UserId,
    pub session: Option<SessionId>,
}

impl Describe for ListUserSessions {
    fn access(&self) -> Access {
        Access::Requires(Permission::ReadUser(self.id.clone()))
    }
}

impl Query for ListUserSessions {
    type Output = Vec<UserSession>;
}
