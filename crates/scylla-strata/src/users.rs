use crate::access::CallerOf;
use scylla_auth::authz::{GrantRepository, Principal, ensure_owner_remains};
use scylla_core::application::guards::user_only;
use scylla_core::application::pagination::PaginatedResult;
use scylla_core::application::user::reset::{NewReset, ResetLinks as Links, SendPasswordReset};
use scylla_core::application::user::{
    AccountRepository, ChangePassword, CreateUser, DeleteAccount, DeleteUser, GetMe, GetUser,
    GetUserByEmail, GetUserByUsername, ListUserAccess, ListUserSessions, ListUsers,
    PasswordResetDelivery, PasswordResetSender, PasswordResetUseCases, RevokeUserSession,
    RevokeUserSessions, SetUserActive, UpdateUser, UpdateUserEmail, UserRepository, UserSession,
    UserUseCases, session_not_found, wrong_current_password, wrong_password,
};
use scylla_core::application::{HashService, PrincipalAccess};
use scylla_domain::domain::caller::CallerContext;
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::SessionId;
use scylla_domain::domain::user::User;
use scylla_extension::Describe;
use std::sync::Arc;
use strata::{BuildCx, BuildError, Component, Resource};

#[derive(Clone)]
pub struct ResetLinkBase(pub Links);

impl Resource for ResetLinkBase {
    const SECTION: &'static str = "reset";
    async fn build(_: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Err(BuildError::msg(
            "the reset links are built by the binary: give them with `Builder::provide`",
        ))
    }
}

pub struct UserResetLinks(Links);

impl Component for UserResetLinks {
    async fn build(cx: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Ok(UserResetLinks(cx.resource::<ResetLinkBase>().await?.0))
    }
}

#[strata::module(section = "users")]
pub trait Users: crate::Access {
    #[component]
    type UserRepo = Arc<dyn UserRepository>;

    #[component]
    type UserGrants = Arc<dyn GrantRepository>;

    #[component]
    type UserHasher = Arc<dyn HashService>;

    #[component]
    type UserAccounts = Arc<dyn AccountRepository>;

    #[component]
    type ResetSender = Arc<dyn PasswordResetSender>;

    #[component]
    type ResetLinks = UserResetLinks;

    fn call_session(&self, named: Option<SessionId>) -> Option<SessionId> {
        named.or_else(|| self.ctx().session().cloned())
    }

    fn user_rules(&self) -> UserUseCases {
        UserUseCases::new(
            self.user_repo().clone(),
            self.user_grants().clone(),
            self.user_accounts().clone(),
        )
    }

    fn reset_rules(&self) -> PasswordResetUseCases {
        PasswordResetUseCases::new(
            self.user_accounts().clone(),
            self.reset_sender().clone(),
            self.reset_links().0.clone(),
        )
    }

    #[usecase(write, access = cmd.access())]
    async fn create_user(&self, cmd: CreateUser) -> DomainResult<User> {
        let password_hash = self.user_hasher().hash(&cmd.password).await?;
        let user = User::create(cmd.username, Some(cmd.email), password_hash)
            .with_display_name(cmd.display_name);
        self.user_repo().create(&user).await
    }

    #[usecase(read, access = query.access())]
    async fn get_user(&self, query: GetUser) -> DomainResult<User> {
        self.user_repo().find_by_id(&query.id).await
    }

    #[usecase(read, access = GetMe.access())]
    #[allow(unused_variables)]
    async fn get_me(&self, query: GetMe) -> DomainResult<User> {
        let id = user_only(self.ctx().caller())?;
        self.user_repo().find_by_id(&id).await
    }

    #[usecase(read, access = query.access())]
    async fn get_user_by_username(&self, query: GetUserByUsername) -> DomainResult<User> {
        self.user_repo().find_by_username(&query.username).await
    }

    #[usecase(read, access = query.access())]
    async fn get_user_by_email(&self, query: GetUserByEmail) -> DomainResult<User> {
        self.user_repo().find_by_email(&query.email).await
    }

    #[usecase(write, access = cmd.access())]
    async fn update_user(&self, cmd: UpdateUser) -> DomainResult<User> {
        let mut user = self.user_repo().find_by_id(&cmd.id).await?;
        if let Some(username) = cmd.username {
            user.update_username(username)?;
        }
        if let Some(display_name) = cmd.display_name {
            user.set_display_name(display_name);
        }
        if let Some(email) = cmd.email {
            user.update_email(email);
        }
        self.user_repo().update(&user).await
    }

    #[usecase(write, access = cmd.access())]
    async fn update_user_email(&self, cmd: UpdateUserEmail) -> DomainResult<User> {
        let mut user = self.user_repo().find_by_id(&cmd.id).await?;
        if user.email().is_some_and(|email| email != &cmd.email) {
            return Err(DomainError::business_rule("the user already has an email"));
        }
        user.update_email(cmd.email);
        self.user_repo().update(&user).await
    }

    #[usecase(write, access = cmd.access())]
    async fn delete_user(&self, cmd: DeleteUser) -> DomainResult<()> {
        let user = self.user_repo().find_by_id(&cmd.id).await?;
        let principal = Principal::User(user.id().clone());
        let grants = self.user_grants().list_all().await?;
        ensure_owner_remains(&grants, |g| g.principal == principal)?;
        self.user_repo().delete(&user).await
    }

    #[usecase(read, access = query.access())]
    async fn list_users(&self, query: ListUsers) -> DomainResult<PaginatedResult<User>> {
        self.user_repo().list_all(query.pagination.as_ref()).await
    }

    #[usecase(write, access = cmd.access())]
    async fn change_password(&self, cmd: ChangePassword) -> DomainResult<User> {
        let id = user_only(self.ctx().caller())?;
        let mut user = self.user_repo().find_by_id(&id).await?;
        if !self
            .user_hasher()
            .verify(&cmd.current_password, user.password_hash())
            .await?
        {
            return Err(wrong_current_password());
        }
        user.set_password_hash(self.user_hasher().hash(&cmd.new_password).await?);
        self.user_accounts()
            .update_signed_out(&user, self.call_session(cmd.session).as_ref())
            .await
    }

    #[usecase(write, access = cmd.access())]
    async fn send_password_reset(
        &self,
        cmd: SendPasswordReset,
    ) -> DomainResult<PasswordResetDelivery> {
        let user = self.user_repo().find_by_id(&cmd.id).await?;
        if !user.is_active() {
            return Err(DomainError::business_rule("The account is inactive"));
        }
        let Some(email) = user.email() else {
            return Err(DomainError::business_rule("The account has no email"));
        };
        let NewReset { reset, message } = self.reset_rules().new_reset(&user, email)?;
        self.user_accounts().issue_reset(&reset, None).await?;
        self.reset_sender().send(&message).await?;
        Ok(self.reset_sender().delivery())
    }

    #[usecase(write, access = cmd.access())]
    async fn revoke_user_sessions(&self, cmd: RevokeUserSessions) -> DomainResult<u64> {
        self.user_repo().find_by_id(&cmd.id).await?;
        let keep = match self.ctx().caller() {
            CallerContext::User(caller) if caller == &cmd.id => self.call_session(cmd.session),
            _ => None,
        };
        self.user_accounts()
            .revoke_sessions(&cmd.id, keep.as_ref())
            .await
    }

    #[usecase(write, access = cmd.access())]
    async fn revoke_user_session(&self, cmd: RevokeUserSession) -> DomainResult<()> {
        let session = self
            .user_accounts()
            .list_sessions(&cmd.id)
            .await?
            .into_iter()
            .find(|s| s.id == cmd.session_id)
            .ok_or_else(|| session_not_found(&cmd.session_id))?;
        if self
            .user_accounts()
            .revoke_session(&cmd.id, &session.id)
            .await?
        {
            Ok(())
        } else {
            Err(session_not_found(&session.id))
        }
    }

    #[usecase(write, access = cmd.access())]
    async fn set_user_active(&self, cmd: SetUserActive) -> DomainResult<User> {
        if self.ctx().caller() == &CallerContext::User(cmd.id.clone()) {
            return Err(DomainError::business_rule(
                "You cannot change the active state of your own account",
            ));
        }
        let mut user = self.user_repo().find_by_id(&cmd.id).await?;
        if user.is_active() && !cmd.is_active {
            self.user_rules()
                .ensure_an_active_admin_remains(user.id())
                .await?;
        }
        if !user.set_active(cmd.is_active) {
            Ok(user)
        } else if user.is_active() {
            self.user_repo().update(&user).await
        } else {
            self.user_accounts().update_signed_out(&user, None).await
        }
    }

    #[usecase(write, access = cmd.access())]
    async fn delete_account(&self, cmd: DeleteAccount) -> DomainResult<()> {
        let id = user_only(self.ctx().caller())?;
        let user = self.user_repo().find_by_id(&id).await?;
        if !self
            .user_hasher()
            .verify(&cmd.password, user.password_hash())
            .await?
        {
            return Err(wrong_password());
        }
        self.user_rules()
            .ensure_owner_remains_without(&user)
            .await?;
        self.user_repo().delete(&user).await
    }

    #[usecase(read, access = query.access())]
    async fn list_user_access(&self, query: ListUserAccess) -> DomainResult<Vec<PrincipalAccess>> {
        self.user_repo().find_by_id(&query.id).await?;
        self.user_accounts().list_access(&query.id).await
    }

    #[usecase(read, access = query.access())]
    async fn list_user_sessions(&self, query: ListUserSessions) -> DomainResult<Vec<UserSession>> {
        self.user_repo().find_by_id(&query.id).await?;
        let current = self.call_session(query.session);
        let mut sessions = self.user_accounts().list_sessions(&query.id).await?;
        for session in &mut sessions {
            session.current = current.as_ref() == Some(&session.id);
        }
        Ok(sessions)
    }
}
