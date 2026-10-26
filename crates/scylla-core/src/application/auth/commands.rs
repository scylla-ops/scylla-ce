//! The session's writes. Each command, its access and its payload types. The strata module of the
//! slice runs it.

use crate::domain::session::{Session, SessionClient};
use crate::domain::user::Password;
use scylla_extension::{Access, Command, Describe, Draft};

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

/// One pass over the sessions whose time is over. It has no resource for Cedar, so it is
/// `Authenticated` and the use case refuses a caller that is not a service. The output is the count.
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
