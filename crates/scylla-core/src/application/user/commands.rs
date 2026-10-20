//! The user's writes. One block per command, in the order it runs: the struct, its
//! access, its payload types, what `Prepare` builds, what `Persist` writes.

use super::{UserSession, UserUseCases, session_not_found, wrong_current_password, wrong_password};
use crate::application::actions::user_only;
use crate::domain::caller::CallerContext;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{SessionId, UserId};
use crate::domain::permission::Permission;
use crate::domain::user::{DisplayName, Email, Password, User, Username};
use async_trait::async_trait;
use scylla_auth::authz::{Principal, ensure_owner_remains};
use scylla_extension::{
    Access, Authorized, Command, Committed, Deleted, Describe, Draft, Persist, Prepare, Prepared,
    Run,
};

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

#[async_trait]
impl Run<Prepare<CreateUser>> for UserUseCases {
    async fn run(&self, input: Authorized<CreateUser>) -> DomainResult<Prepared<CreateUser>> {
        let cmd = input.command();
        let password_hash = self.hash_service.hash(&cmd.password).await?;
        let user = User::create(cmd.username.clone(), Some(cmd.email.clone()), password_hash)
            .with_display_name(cmd.display_name.clone());
        Ok(input.prepared(Draft::new(user)))
    }
}

#[async_trait]
impl Run<Persist<CreateUser>> for UserUseCases {
    async fn run(&self, input: Prepared<CreateUser>) -> DomainResult<Committed<CreateUser>> {
        input
            .commit(async |draft| self.user_repo.create(&draft.into_inner()).await)
            .await
    }
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

#[async_trait]
impl Run<Prepare<UpdateUser>> for UserUseCases {
    async fn run(&self, input: Authorized<UpdateUser>) -> DomainResult<Prepared<UpdateUser>> {
        let cmd = input.command();
        let mut user = self.user_repo.find_by_id(&cmd.id).await?;
        if let Some(username) = &cmd.username {
            user.update_username(username.clone())?;
        }
        if let Some(display_name) = &cmd.display_name {
            user.set_display_name(display_name.clone());
        }
        if let Some(email) = &cmd.email {
            user.update_email(email.clone());
        }
        Ok(input.prepared(Draft::new(user)))
    }
}

#[async_trait]
impl Run<Persist<UpdateUser>> for UserUseCases {
    async fn run(&self, input: Prepared<UpdateUser>) -> DomainResult<Committed<UpdateUser>> {
        input
            .commit(async |draft| self.user_repo.update(&draft.into_inner()).await)
            .await
    }
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

#[async_trait]
impl Run<Prepare<UpdateUserEmail>> for UserUseCases {
    async fn run(
        &self,
        input: Authorized<UpdateUserEmail>,
    ) -> DomainResult<Prepared<UpdateUserEmail>> {
        let cmd = input.command();
        let mut user = self.user_repo.find_by_id(&cmd.id).await?;
        if user.email().is_some_and(|email| email != &cmd.email) {
            return Err(DomainError::business_rule("the user already has an email"));
        }
        user.update_email(cmd.email.clone());
        Ok(input.prepared(Draft::new(user)))
    }
}

#[async_trait]
impl Run<Persist<UpdateUserEmail>> for UserUseCases {
    async fn run(
        &self,
        input: Prepared<UpdateUserEmail>,
    ) -> DomainResult<Committed<UpdateUserEmail>> {
        input
            .commit(async |draft| self.user_repo.update(&draft.into_inner()).await)
            .await
    }
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

#[async_trait]
impl Run<Prepare<DeleteUser>> for UserUseCases {
    async fn run(&self, input: Authorized<DeleteUser>) -> DomainResult<Prepared<DeleteUser>> {
        let user = self.user_repo.find_by_id(&input.command().id).await?;
        let principal = Principal::User(user.id().clone());
        let grants = self.grant_repo.list_all().await?;
        ensure_owner_remains(&grants, |g| g.principal == principal)?;
        Ok(input.prepared(user))
    }
}

#[async_trait]
impl Run<Persist<DeleteUser>> for UserUseCases {
    async fn run(&self, input: Prepared<DeleteUser>) -> DomainResult<Committed<DeleteUser>> {
        input
            .commit(async |user| {
                self.user_repo.delete(&user).await?;
                Ok(Deleted::new(user))
            })
            .await
    }
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

#[async_trait]
impl Run<Prepare<ChangePassword>> for UserUseCases {
    async fn run(
        &self,
        input: Authorized<ChangePassword>,
    ) -> DomainResult<Prepared<ChangePassword>> {
        let id = user_only(input.caller())?;
        let cmd = input.command();
        let mut user = self.user_repo.find_by_id(&id).await?;
        if !self
            .hash_service
            .verify(&cmd.current_password, user.password_hash())
            .await?
        {
            return Err(wrong_current_password());
        }
        user.set_password_hash(self.hash_service.hash(&cmd.new_password).await?);
        Ok(input.prepared(Draft::new(user)))
    }
}

/// Revokes the other sessions of the caller and cancels its reset links.
#[async_trait]
impl Run<Persist<ChangePassword>> for UserUseCases {
    async fn run(
        &self,
        input: Prepared<ChangePassword>,
    ) -> DomainResult<Committed<ChangePassword>> {
        let keep = input.command().session.clone();
        input
            .commit(async |draft| {
                self.accounts
                    .update_signed_out(&draft.into_inner(), keep.as_ref())
                    .await
            })
            .await
    }
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

#[async_trait]
impl Run<Prepare<RevokeUserSessions>> for UserUseCases {
    async fn run(
        &self,
        input: Authorized<RevokeUserSessions>,
    ) -> DomainResult<Prepared<RevokeUserSessions>> {
        let cmd = input.command();
        self.user_repo.find_by_id(&cmd.id).await?;
        let keep = match input.caller() {
            CallerContext::User(caller) if caller == &cmd.id => cmd.session.clone(),
            _ => None,
        };
        Ok(input.prepared(keep))
    }
}

#[async_trait]
impl Run<Persist<RevokeUserSessions>> for UserUseCases {
    async fn run(
        &self,
        input: Prepared<RevokeUserSessions>,
    ) -> DomainResult<Committed<RevokeUserSessions>> {
        let id = input.command().id.clone();
        input
            .commit(async |keep| self.accounts.revoke_sessions(&id, keep.as_ref()).await)
            .await
    }
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

#[async_trait]
impl Run<Prepare<RevokeUserSession>> for UserUseCases {
    async fn run(
        &self,
        input: Authorized<RevokeUserSession>,
    ) -> DomainResult<Prepared<RevokeUserSession>> {
        let cmd = input.command();
        let session = self
            .accounts
            .list_sessions(&cmd.id)
            .await?
            .into_iter()
            .find(|s| s.id == cmd.session_id)
            .ok_or_else(|| session_not_found(&cmd.session_id))?;
        Ok(input.prepared(session))
    }
}

/// A session that a concurrent call revoked, or that expired since `Prepare`, gives `NotFound`.
#[async_trait]
impl Run<Persist<RevokeUserSession>> for UserUseCases {
    async fn run(
        &self,
        input: Prepared<RevokeUserSession>,
    ) -> DomainResult<Committed<RevokeUserSession>> {
        let id = input.command().id.clone();
        input
            .commit(async |session| {
                if self.accounts.revoke_session(&id, &session.id).await? {
                    Ok(Deleted::new(session))
                } else {
                    Err(session_not_found(&session.id))
                }
            })
            .await
    }
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

#[async_trait]
impl Run<Prepare<SetUserActive>> for UserUseCases {
    async fn run(&self, input: Authorized<SetUserActive>) -> DomainResult<Prepared<SetUserActive>> {
        let cmd = input.command();
        if input.caller() == &CallerContext::User(cmd.id.clone()) {
            return Err(DomainError::business_rule(
                "You cannot change the active state of your own account",
            ));
        }
        let mut user = self.user_repo.find_by_id(&cmd.id).await?;
        if user.is_active() && !cmd.is_active {
            self.ensure_an_active_admin_remains(user.id()).await?;
        }
        let changed = user.set_active(cmd.is_active);
        Ok(input.prepared((Draft::new(user), changed)))
    }
}

/// A deactivation revokes every session of the user and cancels its reset links.
#[async_trait]
impl Run<Persist<SetUserActive>> for UserUseCases {
    async fn run(&self, input: Prepared<SetUserActive>) -> DomainResult<Committed<SetUserActive>> {
        input
            .commit(async |(draft, changed)| {
                let user = draft.into_inner();
                if !changed {
                    Ok(user)
                } else if user.is_active() {
                    self.user_repo.update(&user).await
                } else {
                    self.accounts.update_signed_out(&user, None).await
                }
            })
            .await
    }
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

#[async_trait]
impl Run<Prepare<DeleteAccount>> for UserUseCases {
    async fn run(&self, input: Authorized<DeleteAccount>) -> DomainResult<Prepared<DeleteAccount>> {
        let id = user_only(input.caller())?;
        let user = self.user_repo.find_by_id(&id).await?;
        if !self
            .hash_service
            .verify(&input.command().password, user.password_hash())
            .await?
        {
            return Err(wrong_password());
        }
        self.ensure_owner_remains_without(&user).await?;
        Ok(input.prepared(user))
    }
}

#[async_trait]
impl Run<Persist<DeleteAccount>> for UserUseCases {
    async fn run(&self, input: Prepared<DeleteAccount>) -> DomainResult<Committed<DeleteAccount>> {
        input
            .commit(async |user| {
                self.user_repo.delete(&user).await?;
                Ok(Deleted::new(user))
            })
            .await
    }
}
