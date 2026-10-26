use crate::access::CallerOf;
use scylla_auth::authz::{GrantRepository, Principal, ensure_owner_remains};
use scylla_core::application::guards::user_only;
use scylla_core::application::pagination::{PaginatedResult, PaginationParams};
use scylla_core::application::user::reset::{NewReset, ResetLinks as Links};
use scylla_core::application::user::{
    AccountRepository, PasswordResetDelivery, PasswordResetSender, PasswordResetUseCases,
    UserRepository, UserSession, UserUseCases, session_not_found, wrong_current_password,
    wrong_password,
};
use scylla_core::application::{HashService, PrincipalAccess};
use scylla_domain::domain::caller::CallerContext;
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{SessionId, UserId};
use scylla_domain::domain::permission::Permission;
use scylla_domain::domain::user::{DisplayName, Email, Password, User, Username};
use scylla_extension::Access;
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

/// The access of `update_user`: an email also needs `createUser`.
fn update_user_access(id: &UserId, email: Option<&Email>) -> Access {
    let update = Permission::UpdateUser(id.clone());
    if email.is_some() {
        Access::RequiresAll(vec![update, Permission::CreateUser])
    } else {
        Access::Requires(update)
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

    #[usecase(write, access = Access::Requires(Permission::CreateUser))]
    async fn create_user(
        &self,
        username: Username,
        email: Email,
        password: Password,
        display_name: Option<DisplayName>,
    ) -> DomainResult<User> {
        let password_hash = self.user_hasher().hash(&password).await?;
        let user =
            User::create(username, Some(email), password_hash).with_display_name(display_name);
        self.user_repo().create(&user).await
    }

    #[usecase(read, access = Access::Requires(Permission::ReadUser(id.clone())))]
    async fn get_user(&self, id: UserId) -> DomainResult<User> {
        self.user_repo().find_by_id(&id).await
    }

    /// The account of the caller. It needs no permission. An app or a service has no account.
    #[usecase(read, access = Access::Authenticated)]
    async fn get_me(&self) -> DomainResult<User> {
        let id = user_only(self.ctx().caller())?;
        self.user_repo().find_by_id(&id).await
    }

    /// No RPC calls it. The bootstrap reads back an admin that already exists.
    #[usecase(read, access = Access::Requires(Permission::ListUsers))]
    async fn get_user_by_username(&self, username: Username) -> DomainResult<User> {
        self.user_repo().find_by_username(&username).await
    }

    /// No RPC calls it. The bootstrap finds the admin account by its configured email.
    #[usecase(read, access = Access::Requires(Permission::ListUsers))]
    async fn get_user_by_email(&self, email: Email) -> DomainResult<User> {
        self.user_repo().find_by_email(&email).await
    }

    /// `None` keeps a field. `display_name: Some(None)` removes the display name. An email is the
    /// address of the reset links, thus an email also needs `createUser`, also on the account of
    /// the caller.
    #[usecase(write, access = update_user_access(&id, email.as_ref()))]
    async fn update_user(
        &self,
        id: UserId,
        username: Option<Username>,
        display_name: Option<Option<DisplayName>>,
        email: Option<Email>,
    ) -> DomainResult<User> {
        let mut user = self.user_repo().find_by_id(&id).await?;
        if let Some(username) = username {
            user.update_username(username)?;
        }
        if let Some(display_name) = display_name {
            user.set_display_name(display_name);
        }
        if let Some(email) = email {
            user.update_email(email);
        }
        self.user_repo().update(&user).await
    }

    /// No RPC calls it. The bootstrap gives its email to an admin account that has none. An
    /// account with another email keeps it.
    #[usecase(write, access = Access::Requires(Permission::UpdateUser(id.clone())))]
    async fn update_user_email(&self, id: UserId, email: Email) -> DomainResult<User> {
        let mut user = self.user_repo().find_by_id(&id).await?;
        if user.email().is_some_and(|current| current != &email) {
            return Err(DomainError::business_rule("the user already has an email"));
        }
        user.update_email(email);
        self.user_repo().update(&user).await
    }

    #[usecase(write, access = Access::Requires(Permission::DeleteUser(id.clone())))]
    async fn delete_user(&self, id: UserId) -> DomainResult<()> {
        let user = self.user_repo().find_by_id(&id).await?;
        let principal = Principal::User(user.id().clone());
        let grants = self.user_grants().list_all().await?;
        ensure_owner_remains(&grants, |g| g.principal == principal)?;
        self.user_repo().delete(&user).await
    }

    #[usecase(read, access = Access::Requires(Permission::ListUsers))]
    async fn list_users(
        &self,
        pagination: Option<PaginationParams>,
    ) -> DomainResult<PaginatedResult<User>> {
        self.user_repo().list_all(pagination.as_ref()).await
    }

    /// The session of the call (in the context) stays open.
    #[usecase(write, access = Access::Authenticated)]
    async fn change_password(
        &self,
        current_password: Password,
        new_password: Password,
    ) -> DomainResult<User> {
        let id = user_only(self.ctx().caller())?;
        let mut user = self.user_repo().find_by_id(&id).await?;
        if !self
            .user_hasher()
            .verify(&current_password, user.password_hash())
            .await?
        {
            return Err(wrong_current_password());
        }
        user.set_password_hash(self.user_hasher().hash(&new_password).await?);
        self.user_accounts()
            .update_signed_out(&user, self.ctx().session())
            .await
    }

    /// The delivery is inline: the caller sees an error when the send fails.
    #[usecase(write, access = Access::Requires(Permission::UpdateUser(id.clone())))]
    async fn send_password_reset(&self, id: UserId) -> DomainResult<PasswordResetDelivery> {
        let user = self.user_repo().find_by_id(&id).await?;
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

    /// The session of the call (in the context) stays open only when the user is the caller.
    #[usecase(write, access = Access::Requires(Permission::UpdateUser(id.clone())))]
    async fn revoke_user_sessions(&self, id: UserId) -> DomainResult<u64> {
        self.user_repo().find_by_id(&id).await?;
        let keep = match self.ctx().caller() {
            CallerContext::User(caller) if caller == &id => self.ctx().session().cloned(),
            _ => None,
        };
        self.user_accounts()
            .revoke_sessions(&id, keep.as_ref())
            .await
    }

    /// Revokes one session of the user. The session of the call can be revoked: the caller is
    /// then signed out. An unknown id, a session of another user and an expired session give the
    /// same `NotFound`.
    #[usecase(write, access = Access::Requires(Permission::UpdateUser(id.clone())))]
    async fn revoke_user_session(&self, id: UserId, session_id: SessionId) -> DomainResult<()> {
        let session = self
            .user_accounts()
            .list_sessions(&id)
            .await?
            .into_iter()
            .find(|s| s.id == session_id)
            .ok_or_else(|| session_not_found(&session_id))?;
        if self
            .user_accounts()
            .revoke_session(&id, &session.id)
            .await?
        {
            Ok(())
        } else {
            Err(session_not_found(&session.id))
        }
    }

    /// The account of the caller gets no exemption: the caller cannot change its own flag. The
    /// current value writes nothing.
    #[usecase(write, access = Access::Requires(Permission::UpdateUser(id.clone())))]
    async fn set_user_active(&self, id: UserId, is_active: bool) -> DomainResult<User> {
        if self.ctx().caller() == &CallerContext::User(id.clone()) {
            return Err(DomainError::business_rule(
                "You cannot change the active state of your own account",
            ));
        }
        let mut user = self.user_repo().find_by_id(&id).await?;
        if user.is_active() && !is_active {
            self.user_rules()
                .ensure_an_active_admin_remains(user.id())
                .await?;
        }
        if !user.set_active(is_active) {
            Ok(user)
        } else if user.is_active() {
            self.user_repo().update(&user).await
        } else {
            self.user_accounts().update_signed_out(&user, None).await
        }
    }

    /// The deletion of the account of the caller, with the effects of `delete_user`.
    #[usecase(write, access = Access::Authenticated)]
    async fn delete_account(&self, password: Password) -> DomainResult<()> {
        let id = user_only(self.ctx().caller())?;
        let user = self.user_repo().find_by_id(&id).await?;
        if !self
            .user_hasher()
            .verify(&password, user.password_hash())
            .await?
        {
            return Err(wrong_password());
        }
        self.user_rules()
            .ensure_owner_remains_without(&user)
            .await?;
        self.user_repo().delete(&user).await
    }

    #[usecase(read, access = Access::Requires(Permission::ReadUser(id.clone())))]
    async fn list_user_access(&self, id: UserId) -> DomainResult<Vec<PrincipalAccess>> {
        self.user_repo().find_by_id(&id).await?;
        self.user_accounts().list_access(&id).await
    }

    /// The output marks the session of the call (in the context) `current`.
    #[usecase(read, access = Access::Requires(Permission::ReadUser(id.clone())))]
    async fn list_user_sessions(&self, id: UserId) -> DomainResult<Vec<UserSession>> {
        self.user_repo().find_by_id(&id).await?;
        let current = self.ctx().session().cloned();
        let mut sessions = self.user_accounts().list_sessions(&id).await?;
        for session in &mut sessions {
            session.current = current.as_ref() == Some(&session.id);
        }
        Ok(sessions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_email_change_also_needs_create_user() {
        let id = UserId::new("u1");
        let email = Email::new("u1@example.com").unwrap();

        assert_eq!(
            update_user_access(&id, Some(&email)).to_string(),
            "updateUser+createUser"
        );
        assert_eq!(update_user_access(&id, None).to_string(), "updateUser");
    }
}
