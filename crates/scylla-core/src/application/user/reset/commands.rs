//! The reset links' writes. Each command, its access and its payload types. The strata module of
//! the slice runs it.

use super::{NewReset, PasswordResetDelivery};
use crate::domain::ids::UserId;
use crate::domain::permission::Permission;
use crate::domain::user::{Email, Password, PasswordReset, ResetToken, User};
use scylla_extension::{Access, Command, Describe, Draft};

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
