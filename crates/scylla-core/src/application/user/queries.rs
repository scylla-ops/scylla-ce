//! The user's reads. One block per query, in the order it runs: the struct, its
//! access, its output type, what `Fetch` reads.

use super::{UserSession, UserUseCases};
use crate::application::actions::user_only;
use crate::application::grant::PrincipalAccess;
use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::domain::errors::DomainResult;
use crate::domain::ids::{SessionId, UserId};
use crate::domain::permission::Permission;
use crate::domain::user::{Email, User, Username};
use async_trait::async_trait;
use scylla_extension::{Access, Authorized, Describe, Fetch, Fetched, Query, Run};

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

#[async_trait]
impl Run<Fetch<GetUser>> for UserUseCases {
    async fn run(&self, input: Authorized<GetUser>) -> DomainResult<Fetched<GetUser>> {
        let user = self.user_repo.find_by_id(&input.command().id).await?;
        Ok(input.fetched(user))
    }
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

#[async_trait]
impl Run<Fetch<GetMe>> for UserUseCases {
    async fn run(&self, input: Authorized<GetMe>) -> DomainResult<Fetched<GetMe>> {
        let id = user_only(input.caller())?;
        let user = self.user_repo.find_by_id(&id).await?;
        Ok(input.fetched(user))
    }
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

#[async_trait]
impl Run<Fetch<GetUserByUsername>> for UserUseCases {
    async fn run(
        &self,
        input: Authorized<GetUserByUsername>,
    ) -> DomainResult<Fetched<GetUserByUsername>> {
        let user = self
            .user_repo
            .find_by_username(&input.command().username)
            .await?;
        Ok(input.fetched(user))
    }
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

#[async_trait]
impl Run<Fetch<GetUserByEmail>> for UserUseCases {
    async fn run(
        &self,
        input: Authorized<GetUserByEmail>,
    ) -> DomainResult<Fetched<GetUserByEmail>> {
        let user = self.user_repo.find_by_email(&input.command().email).await?;
        Ok(input.fetched(user))
    }
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

#[async_trait]
impl Run<Fetch<ListUsers>> for UserUseCases {
    async fn run(&self, input: Authorized<ListUsers>) -> DomainResult<Fetched<ListUsers>> {
        let page = self
            .user_repo
            .list_all(input.command().pagination.as_ref())
            .await?;
        Ok(input.fetched(page))
    }
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

#[async_trait]
impl Run<Fetch<ListUserAccess>> for UserUseCases {
    async fn run(
        &self,
        input: Authorized<ListUserAccess>,
    ) -> DomainResult<Fetched<ListUserAccess>> {
        let id = &input.command().id;
        self.user_repo.find_by_id(id).await?;
        let access = self.accounts.list_access(id).await?;
        Ok(input.fetched(access))
    }
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

#[async_trait]
impl Run<Fetch<ListUserSessions>> for UserUseCases {
    async fn run(
        &self,
        input: Authorized<ListUserSessions>,
    ) -> DomainResult<Fetched<ListUserSessions>> {
        let cmd = input.command();
        self.user_repo.find_by_id(&cmd.id).await?;
        let mut sessions = self.accounts.list_sessions(&cmd.id).await?;
        for session in &mut sessions {
            session.current = cmd.session.as_ref() == Some(&session.id);
        }
        Ok(input.fetched(sessions))
    }
}
