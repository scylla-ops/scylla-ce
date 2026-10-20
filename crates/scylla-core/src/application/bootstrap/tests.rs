//! The bootstrap driver on stub ports: which account gets the System admin grant.

use super::*;
use crate::domain::errors::DomainResult;
use crate::domain::ids::{AppId, UserId};
use crate::domain::permission::ResourceRef;
use crate::domain::role::{RoleDescription, RoleDisplayName};
use crate::test_support::authz::{RecordingPermissionService, actions};
use crate::test_support::stubs::{
    StubAccounts, StubGrants, StubHash, StubRegistry, StubRoles, StubUsers,
};
use crate::test_support::users::UserBuilder;
use async_trait::async_trait;
use scylla_auth::authz::{
    AuthzEntityProvider, FULL_CONTROL, Grant, ResourceAncestors, Role, RoleKind, SYSTEM_ADMIN_ROLE,
    ScopeKind,
};

const USERNAME: &str = "admin";
const EMAIL: &str = "admin@example.com";

/// A System grant reads no ancestor.
struct NoAncestry;

#[async_trait]
impl AuthzEntityProvider for NoAncestry {
    async fn resource_ancestors(&self, _: &ResourceRef) -> DomainResult<ResourceAncestors> {
        unreachable!("a System grant has no ancestor to read")
    }
    async fn app_is_active(&self, _: &AppId) -> DomainResult<bool> {
        unreachable!("no app in the bootstrap")
    }
    async fn policy_version(&self) -> DomainResult<i64> {
        Ok(0)
    }
}

struct Lab {
    bootstrap: BootstrapUseCases,
    users: Arc<StubUsers>,
    grants: Arc<StubGrants>,
}

impl Lab {
    async fn run(&self) -> Result<(), BootstrapError> {
        self.bootstrap
            .bootstrap_admin(
                Username::new(USERNAME).unwrap(),
                Email::new(EMAIL).unwrap(),
                Password::new("SecurePass123!").unwrap(),
                RoleName::new(SYSTEM_ADMIN_ROLE).unwrap(),
            )
            .await
    }

    fn granted(&self) -> Vec<UserId> {
        self.grants
            .created()
            .into_iter()
            .map(|g| match g.principal {
                Principal::User(id) => id,
                Principal::App(id) => panic!("the bootstrap granted the app {id}"),
            })
            .collect()
    }

    fn by_username(&self, username: &str) -> User {
        self.users
            .rows()
            .into_values()
            .find(|u| u.username().as_str() == username)
            .unwrap()
    }
}

fn system_admin() -> Role {
    Role {
        id: SYSTEM_ADMIN_ROLE.to_string(),
        key: Some(SYSTEM_ADMIN_ROLE.to_string()),
        name: RoleDisplayName::new("System Admin").unwrap(),
        description: RoleDescription::new("").unwrap(),
        scope: ScopeKind::System,
        kind: RoleKind::Admin,
        owner_org: None,
        builtin: true,
        permissions: vec![FULL_CONTROL.to_string()],
        version: 0,
    }
}

fn lab(existing: Vec<User>) -> Lab {
    let users = Arc::new(StubUsers::with(existing));
    let grants = Arc::new(StubGrants::new(Vec::<Grant>::new()));
    let actions = Arc::new(actions(Arc::new(RecordingPermissionService::new())));
    let user_uc = Arc::new(UserUseCases::new(
        users.clone(),
        grants.clone(),
        Arc::new(StubHash::passwords()),
        Arc::new(StubAccounts::new(users.clone())),
    ));
    let grant_uc = Arc::new(GrantUseCases::new(
        grants.clone(),
        Arc::new(StubRoles::new(vec![system_admin()])),
        Arc::new(StubRegistry::default()),
        Arc::new(NoAncestry),
    ));
    Lab {
        bootstrap: BootstrapUseCases::new(actions, user_uc, grant_uc),
        users,
        grants,
    }
}

#[tokio::test]
async fn a_new_account_is_created_with_the_email_and_granted() {
    let lab = lab(Vec::new());

    lab.run().await.unwrap();

    let admin = lab.by_username(USERNAME);
    assert_eq!(admin.email().map(Email::as_str), Some(EMAIL));
    assert_eq!(lab.granted(), vec![admin.id().clone()]);
}

#[tokio::test]
async fn the_account_with_the_username_and_the_email_is_granted() {
    let existing = UserBuilder::new(USERNAME).email(EMAIL).build();
    let lab = lab(vec![existing.clone()]);

    lab.run().await.unwrap();

    assert_eq!(lab.users.rows().len(), 1);
    assert_eq!(lab.granted(), vec![existing.id().clone()]);
}

#[tokio::test]
async fn the_email_of_another_account_stops_the_bootstrap_without_a_grant() {
    let other = UserBuilder::new("mallory").email(EMAIL).build();
    let lab = lab(vec![other]);

    let err = lab.run().await.unwrap_err();

    assert!(
        matches!(err, BootstrapError::EmailOfAnotherAccount),
        "{err}"
    );
    assert!(lab.granted().is_empty());
    assert_eq!(lab.users.rows().len(), 1);
}

#[tokio::test]
async fn the_username_with_another_email_stops_the_bootstrap_without_a_grant() {
    let existing = UserBuilder::new(USERNAME)
        .email("someone@example.com")
        .build();
    let lab = lab(vec![existing.clone()]);

    let err = lab.run().await.unwrap_err();

    assert!(
        matches!(err, BootstrapError::UsernameWithAnotherEmail),
        "{err}"
    );
    assert!(lab.granted().is_empty());
    assert_eq!(
        lab.by_username(USERNAME).email().map(Email::as_str),
        Some("someone@example.com")
    );
}

#[tokio::test]
async fn the_username_without_an_email_gets_the_email_and_the_grant() {
    let existing = UserBuilder::new(USERNAME).build();
    let lab = lab(vec![existing.clone()]);

    lab.run().await.unwrap();

    let admin = lab.by_username(USERNAME);
    assert_eq!(admin.id(), existing.id());
    assert_eq!(admin.email().map(Email::as_str), Some(EMAIL));
    assert_eq!(lab.granted(), vec![existing.id().clone()]);
}
