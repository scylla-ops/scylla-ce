use crate::application::GrantUseCases;
use crate::application::grant::CreateGrant;
use crate::application::user::{
    CreateUser, GetUserByEmail, GetUserByUsername, UpdateUserEmail, UserUseCases,
};
use crate::domain::caller::{CallerContext, ServiceIdentity};
use crate::domain::errors::DomainError;
use crate::domain::role::RoleName;
use crate::domain::user::{Email, Password, User, Username};
use crate::error::BootstrapError;
use derive_more::Constructor;
use scylla_auth::authz::{Principal, Scope};
use scylla_extension::Actions;
use std::sync::Arc;
use tracing::instrument;

#[derive(Constructor)]
pub struct BootstrapUseCases {
    actions: Arc<Actions>,
    user_uc: Arc<UserUseCases>,
    grant_uc: Arc<GrantUseCases>,
}

impl BootstrapUseCases {
    /// The admin is a new account, the account with the configured email, or the account with the
    /// configured username when it has no email. No other account gets the grant.
    #[instrument(skip_all, fields(username = %username.as_str(), role = %role.as_str()))]
    pub async fn bootstrap_admin(
        &self,
        username: Username,
        email: Email,
        password: Password,
        role: RoleName,
    ) -> Result<(), BootstrapError> {
        let caller = CallerContext::Service(ServiceIdentity::bootstrap());

        let create = CreateUser {
            username: username.clone(),
            email: email.clone(),
            password,
            display_name: None,
        };
        let user = match self.actions.run(&*self.user_uc, &caller, create).await {
            Ok(user) => {
                tracing::info!(
                    user_id = %user.id(),
                    username = %username,
                    "bootstrap user created",
                );
                user
            }
            Err(DomainError::Conflict(_)) => {
                tracing::debug!(username = %username, "bootstrap user already exists");
                self.existing_admin(&caller, &username, email).await?
            }
            Err(e) => return Err(BootstrapError::CreateUser(e)),
        };

        let grant = CreateGrant {
            principal: Principal::User(user.id().clone()),
            role: role.clone(),
            scope: Scope::System,
        };
        self.actions
            .run(&*self.grant_uc, &caller, grant)
            .await
            .map_err(BootstrapError::GrantPermission)?;
        tracing::info!(
            user_id = %user.id(),
            role = %role,
            "bootstrap system grant ensured",
        );
        Ok(())
    }

    async fn existing_admin(
        &self,
        caller: &CallerContext,
        username: &Username,
        email: Email,
    ) -> Result<User, BootstrapError> {
        let by_email = GetUserByEmail {
            email: email.clone(),
        };
        match self.actions.run(&*self.user_uc, caller, by_email).await {
            Ok(user) if user.username() == username => return Ok(user),
            Ok(_) => return Err(BootstrapError::EmailOfAnotherAccount),
            Err(e) if e.is_not_found() => {}
            Err(e) => return Err(BootstrapError::FindUser(e)),
        }

        let by_username = GetUserByUsername {
            username: username.clone(),
        };
        let user = self
            .actions
            .run(&*self.user_uc, caller, by_username)
            .await
            .map_err(BootstrapError::FindUser)?;
        if user.email().is_some() {
            return Err(BootstrapError::UsernameWithAnotherEmail);
        }
        let update = UpdateUserEmail {
            id: user.id().clone(),
            email,
        };
        let user = self
            .actions
            .run(&*self.user_uc, caller, update)
            .await
            .map_err(BootstrapError::UpdateEmail)?;
        tracing::info!(user_id = %user.id(), "bootstrap email set on the existing user");
        Ok(user)
    }
}

#[cfg(test)]
mod tests;
