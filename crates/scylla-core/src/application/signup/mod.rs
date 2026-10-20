pub mod repository;

pub use repository::SignupRepository;

use crate::domain::errors::DomainResult;
use crate::domain::organization::{Organization, OrganizationName};
use crate::domain::role::RoleName;
use crate::domain::user::User;
use scylla_auth::authz::{Grant, ORGANIZATION_ADMIN_ROLE, Principal, Scope};

/// The organization of an account whose user creates one later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoOrganization;

/// A user, its own organization and the grants of the account, the first of which makes the
/// user its admin: what the sign-up of an edition creates in one transaction. An account
/// `without_organization` is a user and its grants only; `provision_user` writes it.
pub struct NewAccount<O = Organization> {
    pub user: User,
    pub organization: O,
    pub grants: Vec<Grant>,
}

impl NewAccount {
    pub fn new(user: User, organization_name: OrganizationName) -> DomainResult<Self> {
        let organization = Organization::create(organization_name, None)?;
        let grant = Grant::new(
            Principal::User(user.id().clone()),
            RoleName::new(ORGANIZATION_ADMIN_ROLE)?,
            Scope::Organization(organization.id().clone()),
        );
        Ok(Self {
            user,
            organization,
            grants: vec![grant],
        })
    }
}

impl NewAccount<NoOrganization> {
    #[must_use]
    pub fn without_organization(user: User) -> Self {
        Self {
            user,
            organization: NoOrganization,
            grants: Vec::new(),
        }
    }
}

impl<O> NewAccount<O> {
    /// One more grant for the user, written in the same transaction.
    #[must_use]
    pub fn with_grant(mut self, role: RoleName, scope: Scope) -> Self {
        self.grants.push(Grant::new(
            Principal::User(self.user.id().clone()),
            role,
            scope,
        ));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::users::user;
    use scylla_auth::authz::ORGANIZATION_CREATOR_ROLE;

    #[test]
    fn an_account_without_an_organization_holds_only_the_grants_it_is_given() {
        let account = NewAccount::without_organization(user("solo"));
        assert!(account.grants.is_empty());

        let account = account.with_grant(
            RoleName::new(ORGANIZATION_CREATOR_ROLE).unwrap(),
            Scope::System,
        );
        assert_eq!(account.grants.len(), 1);
        assert_eq!(
            account.grants[0].principal,
            Principal::User(account.user.id().clone())
        );
        assert_eq!(account.grants[0].scope, Scope::System);
    }

    #[test]
    fn an_account_with_an_organization_makes_the_user_its_admin() {
        let account =
            NewAccount::new(user("founder"), OrganizationName::new("Acme").unwrap()).unwrap();
        assert_eq!(account.grants.len(), 1);
        assert_eq!(account.grants[0].role.as_str(), ORGANIZATION_ADMIN_ROLE);
        assert_eq!(
            account.grants[0].scope,
            Scope::Organization(account.organization.id().clone())
        );
    }
}
