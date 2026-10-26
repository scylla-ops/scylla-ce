use crate::access::CallerOf;
use scylla_core::application::auth::{
    Login, PurgeExpiredSessions, RevokeToken, SessionLookup, ValidateToken, look_up_session,
    new_session,
};
use scylla_core::application::guards::service_only;
use scylla_core::application::user::reset::{
    RequestPasswordReset, ResetLinks as Links, ResetPassword,
};
use scylla_core::application::user::{
    AccountRepository, PasswordResetDelivery, PasswordResetSender, PasswordResetUseCases,
    UserRepository,
};
use scylla_core::application::{HashService, SessionRepository};
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::session::Session;
use scylla_domain::domain::user::{Email, Username, reset_link_invalid};
use scylla_extension::Describe;
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

    #[usecase(write, access = cmd.access())]
    async fn login(&self, cmd: Login) -> DomainResult<Session> {
        let invalid = || DomainError::unauthorized("Invalid credentials");
        let lookup = if cmd.identifier.contains('@') {
            let email = Email::new(&cmd.identifier).map_err(|_| invalid())?;
            self.auth_users().find_by_email(&email).await
        } else {
            let username = Username::new(&cmd.identifier).map_err(|_| invalid())?;
            self.auth_users().find_by_username(&username).await
        };
        let user = lookup.map_err(|_| invalid())?;
        if !self
            .auth_hasher()
            .verify(&cmd.password, user.password_hash())
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

    #[usecase(read, access = query.access())]
    async fn validate_token(&self, query: ValidateToken) -> DomainResult<bool> {
        if query.token.is_empty() {
            return Ok(false);
        }
        Ok(matches!(
            look_up_session(self.auth_sessions().as_ref(), &query.token).await,
            Ok(SessionLookup::Live(_))
        ))
    }

    #[usecase(write, access = cmd.access())]
    async fn revoke_token(&self, cmd: RevokeToken) -> DomainResult<()> {
        self.auth_sessions().delete_by_token(&cmd.token).await
    }

    #[usecase(write, access = cmd.access())]
    #[allow(unused_variables)]
    async fn purge_expired_sessions(&self, cmd: PurgeExpiredSessions) -> DomainResult<u64> {
        service_only(self.ctx().caller())?;
        self.auth_sessions().delete_expired().await
    }

    #[usecase(write, access = cmd.access())]
    async fn request_password_reset(
        &self,
        cmd: RequestPasswordReset,
    ) -> DomainResult<PasswordResetDelivery> {
        let resets = self.resets();
        let staged = match self.auth_users().find_by_email(&cmd.email).await {
            Ok(user) if user.is_active() => Some(resets.new_reset(&user, &cmd.email)?),
            Ok(_) => None,
            Err(e) if e.is_not_found() => None,
            Err(e) => return Err(e),
        };
        if let Some(new) = staged {
            resets.issue_later(new);
        }
        Ok(self.auth_reset_sender().delivery())
    }

    #[usecase(write, access = cmd.access())]
    async fn reset_password(&self, cmd: ResetPassword) -> DomainResult<()> {
        let unknown = |e: DomainError| {
            if e.is_not_found() {
                reset_link_invalid()
            } else {
                e
            }
        };
        let reset = self
            .auth_accounts()
            .find_reset(&cmd.token)
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
        user.set_password_hash(self.auth_hasher().hash(&cmd.new_password).await?);
        self.auth_accounts().redeem_reset(&reset, &user).await?;
        Ok(())
    }
}
