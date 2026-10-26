use crate::access::CallerOf;
use scylla_core::application::auth::{SessionLookup, look_up_session, new_session};
use scylla_core::application::guards::service_only;
use scylla_core::application::user::reset::ResetLinks as Links;
use scylla_core::application::user::{
    AccountRepository, PasswordResetDelivery, PasswordResetSender, PasswordResetUseCases,
    UserRepository,
};
use scylla_core::application::{HashService, SessionRepository};
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::session::Session;
use scylla_domain::domain::user::{Email, Password, ResetToken, Username, reset_link_invalid};
use scylla_extension::Access;
use std::sync::Arc;
use strata::{BuildCx, BuildError, Component};

pub struct AuthResetLinks(Links);

impl Component for AuthResetLinks {
    async fn build(cx: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Ok(AuthResetLinks(
            cx.resource::<crate::ResetLinkBase>().await?.0,
        ))
    }
}

#[strata::module(section = "auth")]
pub trait Auth: crate::Access {
    #[component]
    type AuthUsers = Arc<dyn UserRepository>;

    #[component]
    type AuthSessions = Arc<dyn SessionRepository>;

    #[component]
    type AuthHasher = Arc<dyn HashService>;

    #[component]
    type AuthAccounts = Arc<dyn AccountRepository>;

    #[component]
    type AuthResetSender = Arc<dyn PasswordResetSender>;

    #[component]
    type AuthLinks = AuthResetLinks;

    fn resets(&self) -> PasswordResetUseCases {
        PasswordResetUseCases::new(
            self.auth_accounts().clone(),
            self.auth_reset_sender().clone(),
            self.auth_links().0.clone(),
        )
    }

    /// `identifier` is a username or an email. The new session records the client of the call,
    /// from the context.
    #[usecase(write, access = Access::Public)]
    async fn login(&self, identifier: String, password: Password) -> DomainResult<Session> {
        let invalid = || DomainError::unauthorized("Invalid credentials");
        let lookup = if identifier.contains('@') {
            let email = Email::new(&identifier).map_err(|_| invalid())?;
            self.auth_users().find_by_email(&email).await
        } else {
            let username = Username::new(&identifier).map_err(|_| invalid())?;
            self.auth_users().find_by_username(&username).await
        };
        let user = lookup.map_err(|_| invalid())?;
        if !self
            .auth_hasher()
            .verify(&password, user.password_hash())
            .await?
        {
            return Err(invalid());
        }
        if !user.is_active() {
            return Err(DomainError::unauthorized("User account is inactive"));
        }
        let session = new_session(user.id().clone()).with_client(self.ctx().client());
        self.auth_sessions().create(&session).await
    }

    #[usecase(read, access = Access::Public)]
    async fn validate_token(&self, token: String) -> DomainResult<bool> {
        if token.is_empty() {
            return Ok(false);
        }
        Ok(matches!(
            look_up_session(self.auth_sessions().as_ref(), &token).await,
            Ok(SessionLookup::Live(_))
        ))
    }

    #[usecase(write, access = Access::Public)]
    async fn revoke_token(&self, token: String) -> DomainResult<()> {
        self.auth_sessions().delete_by_token(&token).await
    }

    /// One pass over the sessions whose time is over. It has no resource for the access model,
    /// thus it is `Authenticated` and the use case refuses a caller that is not a service. The
    /// return is the count.
    #[usecase(write, access = Access::Authenticated)]
    async fn purge_expired_sessions(&self) -> DomainResult<u64> {
        service_only(self.ctx().caller())?;
        self.auth_sessions().delete_expired().await
    }

    /// The answer is the same for an unknown email, an inactive account and an active account.
    #[usecase(write, access = Access::Public)]
    async fn request_password_reset(&self, email: Email) -> DomainResult<PasswordResetDelivery> {
        let resets = self.resets();
        let staged = match self.auth_users().find_by_email(&email).await {
            Ok(user) if user.is_active() => Some(resets.new_reset(&user, &email)?),
            Ok(_) => None,
            Err(e) if e.is_not_found() => None,
            Err(e) => return Err(e),
        };
        if let Some(new) = staged {
            resets.issue_later(new);
        }
        Ok(self.auth_reset_sender().delivery())
    }

    #[usecase(write, access = Access::Public)]
    async fn reset_password(&self, token: ResetToken, new_password: Password) -> DomainResult<()> {
        let unknown = |e: DomainError| {
            if e.is_not_found() {
                reset_link_invalid()
            } else {
                e
            }
        };
        let reset = self
            .auth_accounts()
            .find_reset(&token)
            .await
            .map_err(unknown)?;
        reset.ensure_redeemable()?;
        let mut user = self
            .auth_users()
            .find_by_id(reset.user_id())
            .await
            .map_err(unknown)?;
        if !user.is_active() {
            return Err(reset_link_invalid());
        }
        user.set_password_hash(self.auth_hasher().hash(&new_password).await?);
        self.auth_accounts().redeem_reset(&reset, &user).await?;
        Ok(())
    }
}
