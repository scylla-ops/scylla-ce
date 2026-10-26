//! The grant's writes. Each command, its access and its payload types. The strata module of the
//! slice runs it.

use crate::domain::ids::GrantId;
use crate::domain::permission::Permission;
use crate::domain::role::RoleName;
use scylla_auth::authz::{Grant, Principal, Scope};
use scylla_extension::{Access, Command, Deleted, Describe, Draft};

#[derive(Debug)]
pub struct CreateGrant {
    pub principal: Principal,
    pub role: RoleName,
    pub scope: Scope,
}

impl Describe for CreateGrant {
    fn access(&self) -> Access {
        Access::Requires(self.scope.manage_permission())
    }
}

impl Command for CreateGrant {
    type Staged = Draft<Grant>;
    type Committed = Grant;
}

#[derive(Debug)]
pub struct RevokeAllAccess {
    pub principal: Principal,
    pub scope: Scope,
}

impl Describe for RevokeAllAccess {
    fn access(&self) -> Access {
        Access::Requires(self.scope.manage_permission())
    }
}

/// Stages the principal alone: the rows go in one statement, by principal and scope. `Committed`
/// is the number of grants removed.
impl Command for RevokeAllAccess {
    type Staged = Principal;
    type Committed = u64;
}

#[derive(Debug)]
pub struct RevokeGrant {
    pub id: GrantId,
}

impl Describe for RevokeGrant {
    fn access(&self) -> Access {
        Access::Requires(Permission::RevokeGrant(self.id.clone()))
    }
}

/// The last grant of a user on an organization is its membership: revoking it strips the user's
/// grants beneath the organization too, as `RevokeAllAccess` does.
#[derive(Debug)]
pub struct Revocation {
    pub grant: Grant,
    pub whole_organization: bool,
}

/// An unknown id stages `None` and commits nothing: the call succeeds without change.
impl Command for RevokeGrant {
    type Staged = Option<Revocation>;
    type Committed = Option<Deleted<Grant>>;
}
