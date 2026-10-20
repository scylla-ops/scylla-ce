//! The session's writes. One block per command, in the order it runs: the struct, its access,
//! its payload types, what `Prepare` builds, what `Persist` writes.

use super::{AuthUseCases, new_session};
use crate::application::actions::service_only;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::session::{Session, SessionClient};
use crate::domain::user::{Email, Password, Username};
use async_trait::async_trait;
use scylla_extension::{
    Access, Authorized, Command, Committed, Describe, Draft, Persist, Prepare, Prepared, Run,
};

/// `identifier` is a username or an email. `client` is the client of the call: the new session
/// records it.
#[derive(Debug)]
pub struct Login {
    pub identifier: String,
    pub password: Password,
    pub client: SessionClient,
}

impl Describe for Login {
    fn access(&self) -> Access {
        Access::Public
    }
}

impl Command for Login {
    type Staged = Draft<Session>;
    type Committed = Session;
}

/// One opaque error for an unknown account and a wrong password, so a caller cannot probe which
/// accounts exist. Only a caller with the right password learns that the account is inactive.
#[async_trait]
impl Run<Prepare<Login>> for AuthUseCases {
    async fn run(&self, input: Authorized<Login>) -> DomainResult<Prepared<Login>> {
        let cmd = input.command();
        let invalid = || DomainError::unauthorized("Invalid credentials");
        let lookup = if cmd.identifier.contains('@') {
            let email = Email::new(&cmd.identifier).map_err(|_| invalid())?;
            self.user_repo.find_by_email(&email).await
        } else {
            let username = Username::new(&cmd.identifier).map_err(|_| invalid())?;
            self.user_repo.find_by_username(&username).await
        };
        let user = lookup.map_err(|_| invalid())?;

        if !self
            .hash_service
            .verify(&cmd.password, user.password_hash())
            .await?
        {
            return Err(invalid());
        }
        if !user.is_active() {
            return Err(DomainError::unauthorized("User account is inactive"));
        }
        let session = new_session(user.id().clone()).with_client(cmd.client.clone());
        Ok(input.prepared(Draft::new(session)))
    }
}

#[async_trait]
impl Run<Persist<Login>> for AuthUseCases {
    async fn run(&self, input: Prepared<Login>) -> DomainResult<Committed<Login>> {
        input
            .commit(async |draft| self.session_repo.create(&draft.into_inner()).await)
            .await
    }
}

/// No `Debug`: `token` is the session credential.
pub struct RevokeToken {
    pub token: String,
}

impl Describe for RevokeToken {
    fn access(&self) -> Access {
        Access::Public
    }
}

impl Command for RevokeToken {
    type Staged = String;
    type Committed = ();
}

#[async_trait]
impl Run<Prepare<RevokeToken>> for AuthUseCases {
    async fn run(&self, input: Authorized<RevokeToken>) -> DomainResult<Prepared<RevokeToken>> {
        let token = input.command().token.clone();
        Ok(input.prepared(token))
    }
}

#[async_trait]
impl Run<Persist<RevokeToken>> for AuthUseCases {
    async fn run(&self, input: Prepared<RevokeToken>) -> DomainResult<Committed<RevokeToken>> {
        input
            .commit(async |token| self.session_repo.delete_by_token(&token).await)
            .await
    }
}

/// One pass over the sessions whose time is over. It has no resource for Cedar, so it is
/// `Authenticated` and `Prepare` refuses a caller that is not a service. `Committed` is the count.
#[derive(Debug)]
pub struct PurgeExpiredSessions;

impl Describe for PurgeExpiredSessions {
    fn access(&self) -> Access {
        Access::Authenticated
    }
}

impl Command for PurgeExpiredSessions {
    type Staged = ();
    type Committed = u64;
}

#[async_trait]
impl Run<Prepare<PurgeExpiredSessions>> for AuthUseCases {
    async fn run(
        &self,
        input: Authorized<PurgeExpiredSessions>,
    ) -> DomainResult<Prepared<PurgeExpiredSessions>> {
        service_only(input.caller())?;
        Ok(input.prepared(()))
    }
}

#[async_trait]
impl Run<Persist<PurgeExpiredSessions>> for AuthUseCases {
    async fn run(
        &self,
        input: Prepared<PurgeExpiredSessions>,
    ) -> DomainResult<Committed<PurgeExpiredSessions>> {
        input
            .commit(async |()| self.session_repo.delete_expired().await)
            .await
    }
}
