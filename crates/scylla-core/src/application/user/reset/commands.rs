//! The reset links' writes. One block per command, in the order it runs: the struct, its access,
//! its payload types, what `Prepare` builds, what `Persist` writes.

use super::{NewReset, PasswordResetDelivery, PasswordResetUseCases};
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::UserId;
use crate::domain::permission::Permission;
use crate::domain::user::{Email, Password, PasswordReset, ResetToken, User, reset_link_invalid};
use async_trait::async_trait;
use scylla_extension::{
    Access, Authorized, Command, Committed, Describe, Draft, Persist, Prepare, Prepared, Run,
};

/// The answer is the same for an unknown email, an inactive account and an active one.
#[derive(Debug)]
pub struct RequestPasswordReset {
    pub email: Email,
}

impl Describe for RequestPasswordReset {
    fn access(&self) -> Access {
        Access::Public
    }
}

impl Command for RequestPasswordReset {
    type Staged = Option<Draft<NewReset>>;
    type Committed = PasswordResetDelivery;
}

#[async_trait]
impl Run<Prepare<RequestPasswordReset>> for PasswordResetUseCases {
    async fn run(
        &self,
        input: Authorized<RequestPasswordReset>,
    ) -> DomainResult<Prepared<RequestPasswordReset>> {
        let email = &input.command().email;
        let staged = match self.user_repo.find_by_email(email).await {
            Ok(user) if user.is_active() => Some(Draft::new(self.new_reset(&user, email)?)),
            Ok(_) => None,
            Err(e) if e.is_not_found() => None,
            Err(e) => return Err(e),
        };
        Ok(input.prepared(staged))
    }
}

/// The answer does not wait for the store: a task stores and delivers the link after it
/// (`issue_and_deliver`), so its time and its errors are the same for every email.
#[async_trait]
impl Run<Persist<RequestPasswordReset>> for PasswordResetUseCases {
    async fn run(
        &self,
        input: Prepared<RequestPasswordReset>,
    ) -> DomainResult<Committed<RequestPasswordReset>> {
        input
            .commit(async |staged| {
                if let Some(draft) = staged {
                    self.issue_later(draft.into_inner());
                }
                Ok(self.sender.delivery())
            })
            .await
    }
}

#[derive(Debug)]
pub struct ResetPassword {
    pub token: ResetToken,
    pub new_password: Password,
}

impl Describe for ResetPassword {
    fn access(&self) -> Access {
        Access::Public
    }
}

impl Command for ResetPassword {
    type Staged = (PasswordReset, Draft<User>);
    type Committed = ();
}

/// An unknown, used or expired link and an inactive account give one error.
#[async_trait]
impl Run<Prepare<ResetPassword>> for PasswordResetUseCases {
    async fn run(&self, input: Authorized<ResetPassword>) -> DomainResult<Prepared<ResetPassword>> {
        let cmd = input.command();
        let unknown = |e: DomainError| {
            if e.is_not_found() {
                reset_link_invalid()
            } else {
                e
            }
        };
        let reset = self
            .accounts
            .find_reset(&cmd.token)
            .await
            .map_err(unknown)?;
        reset.ensure_redeemable()?;
        let mut user = self
            .user_repo
            .find_by_id(reset.user_id())
            .await
            .map_err(unknown)?;
        if !user.is_active() {
            return Err(reset_link_invalid());
        }
        user.set_password_hash(self.hash_service.hash(&cmd.new_password).await?);
        Ok(input.prepared((reset, Draft::new(user))))
    }
}

#[async_trait]
impl Run<Persist<ResetPassword>> for PasswordResetUseCases {
    async fn run(&self, input: Prepared<ResetPassword>) -> DomainResult<Committed<ResetPassword>> {
        input
            .commit(async |(reset, user)| {
                self.accounts
                    .redeem_reset(&reset, &user.into_inner())
                    .await?;
                Ok(())
            })
            .await
    }
}

/// The delivery is inline: the caller sees an error when the send fails.
#[derive(Debug)]
pub struct SendPasswordReset {
    pub id: UserId,
}

impl Describe for SendPasswordReset {
    fn access(&self) -> Access {
        Access::Requires(Permission::UpdateUser(self.id.clone()))
    }
}

impl Command for SendPasswordReset {
    type Staged = Draft<NewReset>;
    type Committed = PasswordResetDelivery;
}

#[async_trait]
impl Run<Prepare<SendPasswordReset>> for PasswordResetUseCases {
    async fn run(
        &self,
        input: Authorized<SendPasswordReset>,
    ) -> DomainResult<Prepared<SendPasswordReset>> {
        let user = self.user_repo.find_by_id(&input.command().id).await?;
        if !user.is_active() {
            return Err(DomainError::business_rule("The account is inactive"));
        }
        let Some(email) = user.email() else {
            return Err(DomainError::business_rule("The account has no email"));
        };
        let staged = self.new_reset(&user, email)?;
        Ok(input.prepared(Draft::new(staged)))
    }
}

#[async_trait]
impl Run<Persist<SendPasswordReset>> for PasswordResetUseCases {
    async fn run(
        &self,
        input: Prepared<SendPasswordReset>,
    ) -> DomainResult<Committed<SendPasswordReset>> {
        input
            .commit(async |draft| {
                let NewReset { reset, message } = draft.into_inner();
                self.accounts.issue_reset(&reset, None).await?;
                self.sender.send(&message).await?;
                Ok(self.sender.delivery())
            })
            .await
    }
}
