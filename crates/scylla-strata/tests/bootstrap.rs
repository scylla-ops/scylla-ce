//! Tests the admin bootstrap on the Users and Grants modules of Scylla, written with strata: the
//! CE bootstrap tests on stub ports. They check which account gets the System admin grant.

use async_trait::async_trait;
use scylla_auth::authz::PermissionService;
use scylla_auth::authz::{
    AuthzEntityProvider, FULL_CONTROL, Grant, GrantRepository, Principal, ResourceAncestors, Role,
    RoleKind, RoleRepository, SYSTEM_ADMIN_ROLE, Scope, ScopeKind,
};
use scylla_core::application::HashService;
use scylla_core::application::agent::AgentDispatch;
use scylla_core::application::user::reset::ResetLinks;
use scylla_core::application::user::{AccountRepository, PasswordResetSender, UserRepository};
use scylla_core::config::BootstrapConfig;
use scylla_core::error::BootstrapError;
use scylla_core::infrastructure::LogPasswordResetSender;
use scylla_core::test_support::authz::{DenyingPermissionService, RecordingPermissionService};
use scylla_core::test_support::stubs::{
    ScopesByAgent, StubAccounts, StubGrants, StubHash, StubRegistry, StubRoles, StubUsers,
};
use scylla_core::test_support::users::UserBuilder;
use scylla_domain::domain::caller::{CallerContext, ServiceIdentity};
use scylla_domain::domain::errors::DomainResult;
use scylla_domain::domain::ids::{AppId, UserId};
use scylla_domain::domain::permission::{Permission, ResourceRef};
use scylla_domain::domain::role::{RoleDescription, RoleDisplayName};
use scylla_domain::domain::user::{Email, User};
use scylla_strata::{AccessModel, ResetLinkBase, bootstrap_admin};
use std::sync::Arc;
use strata::Edition;

const USERNAME: &str = "admin";
const EMAIL: &str = "admin@example.com";

#[strata::edition(modules(scylla_strata::Access, scylla_strata::Users, scylla_strata::Grants))]
pub struct TestBase;

#[strata::edition(extends = TestBase, overrides(scylla_strata::Access))]
pub struct TestCe;

#[strata::overrides]
impl scylla_strata::Access for TestCe {
    type Permissions = scylla_strata::CedarPermissions;
}

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
    app: TestCe,
    users: Arc<StubUsers>,
    grants: Arc<StubGrants>,
}

impl Lab {
    async fn run(&self) -> Result<(), BootstrapError> {
        let cfg = BootstrapConfig {
            username: USERNAME.to_string(),
            password: "SecurePass123!".to_string(),
            email: EMAIL.to_string(),
        };
        bootstrap_admin(&self.app, &cfg).await
    }

    fn granted(&self) -> Vec<UserId> {
        self.grants
            .created()
            .into_iter()
            .map(|g| {
                assert_eq!(g.scope, Scope::System);
                assert_eq!(g.role.as_str(), SYSTEM_ADMIN_ROLE);
                match g.principal {
                    Principal::User(id) => id,
                    Principal::App(id) => panic!("the bootstrap granted the app {id}"),
                }
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

async fn lab(existing: Vec<User>) -> Lab {
    lab_as(existing, Arc::new(RecordingPermissionService::new())).await
}

async fn lab_as(existing: Vec<User>, permissions: Arc<dyn PermissionService>) -> Lab {
    let users = Arc::new(StubUsers::with(existing));
    let grants = Arc::new(StubGrants::new(Vec::<Grant>::new()));
    let config = strata::Config::new();
    let app = TestCe::builder(&config)
        .provide(AccessModel {
            permissions,
            visibility: Arc::new(ScopesByAgent::everything()),
        })
        .provide_arc::<dyn UserRepository>(users.clone())
        .provide_arc::<dyn GrantRepository>(grants.clone())
        .provide_arc::<dyn HashService>(Arc::new(StubHash::passwords()))
        .provide_arc::<dyn AccountRepository>(Arc::new(StubAccounts::new(users.clone())))
        .provide_arc::<dyn PasswordResetSender>(Arc::new(LogPasswordResetSender))
        .provide(ResetLinkBase(ResetLinks::new(None)))
        .provide_arc::<dyn RoleRepository>(Arc::new(StubRoles::new(vec![system_admin()])))
        .provide_arc::<dyn AgentDispatch>(Arc::new(StubRegistry::default()))
        .provide_arc::<dyn AuthzEntityProvider>(Arc::new(NoAncestry))
        .build()
        .await
        .unwrap();
    Lab { app, users, grants }
}

#[tokio::test]
async fn a_new_account_is_created_with_the_email_and_granted() {
    let lab = lab(Vec::new()).await;

    lab.run().await.unwrap();

    let admin = lab.by_username(USERNAME);
    assert_eq!(admin.email().map(Email::as_str), Some(EMAIL));
    assert_eq!(lab.granted(), vec![admin.id().clone()]);
}

#[tokio::test]
async fn the_account_with_the_username_and_the_email_is_granted() {
    let existing = UserBuilder::new(USERNAME).email(EMAIL).build();
    let lab = lab(vec![existing.clone()]).await;

    lab.run().await.unwrap();

    assert_eq!(lab.users.rows().len(), 1);
    assert_eq!(lab.granted(), vec![existing.id().clone()]);
}

#[tokio::test]
async fn the_email_of_another_account_stops_the_bootstrap_without_a_grant() {
    let other = UserBuilder::new("mallory").email(EMAIL).build();
    let lab = lab(vec![other]).await;

    let err = lab.run().await.unwrap_err();

    assert!(
        matches!(err, BootstrapError::EmailOfAnotherAccount),
        "{err}"
    );
    assert_eq!(
        err.to_string(),
        "the bootstrap email belongs to another account"
    );
    assert_eq!(lab.granted(), Vec::<UserId>::new());
    assert_eq!(lab.users.rows().len(), 1);
}

#[tokio::test]
async fn the_username_with_another_email_stops_the_bootstrap_without_a_grant() {
    let existing = UserBuilder::new(USERNAME)
        .email("someone@example.com")
        .build();
    let lab = lab(vec![existing.clone()]).await;

    let err = lab.run().await.unwrap_err();

    assert!(
        matches!(err, BootstrapError::UsernameWithAnotherEmail),
        "{err}"
    );
    assert_eq!(
        err.to_string(),
        "the bootstrap username belongs to an account with another email"
    );
    assert_eq!(lab.granted(), Vec::<UserId>::new());
    assert_eq!(
        lab.by_username(USERNAME).email().map(Email::as_str),
        Some("someone@example.com")
    );
}

#[tokio::test]
async fn the_username_without_an_email_gets_the_email_and_the_grant() {
    let existing = UserBuilder::new(USERNAME).build();
    let lab = lab(vec![existing.clone()]).await;

    lab.run().await.unwrap();

    let admin = lab.by_username(USERNAME);
    assert_eq!(admin.id(), existing.id());
    assert_eq!(admin.email().map(Email::as_str), Some(EMAIL));
    assert_eq!(lab.granted(), vec![existing.id().clone()]);
}

#[tokio::test]
async fn a_second_run_creates_no_second_account() {
    let lab = lab(Vec::new()).await;

    lab.run().await.unwrap();
    lab.run().await.unwrap();

    let admin = lab.by_username(USERNAME);
    assert_eq!(lab.users.rows().len(), 1);
    assert_eq!(lab.granted(), vec![admin.id().clone(), admin.id().clone()]);
}

#[tokio::test]
async fn each_write_asks_the_permission_of_the_bootstrap_service() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab_as(Vec::new(), permissions.clone()).await;

    lab.run().await.unwrap();

    let bootstrap = CallerContext::Service(ServiceIdentity::bootstrap());
    let checks = permissions.checks();
    assert_ne!(checks, Vec::<(CallerContext, Permission)>::new());
    assert!(checks.iter().all(|(caller, _)| caller == &bootstrap));
}

#[tokio::test]
async fn a_refused_service_writes_nothing() {
    let lab = lab_as(Vec::new(), Arc::new(DenyingPermissionService::new())).await;

    let err = lab.run().await.unwrap_err();

    assert!(matches!(err, BootstrapError::CreateUser(_)), "{err}");
    assert!(lab.users.rows().is_empty());
    assert_eq!(lab.granted(), Vec::<UserId>::new());
}

#[tokio::test]
async fn an_invalid_config_stops_the_bootstrap_before_any_write() {
    let lab = lab(Vec::new()).await;
    let cfg = BootstrapConfig {
        username: USERNAME.to_string(),
        password: "SecurePass123!".to_string(),
        email: "not an email".to_string(),
    };

    let err = bootstrap_admin(&lab.app, &cfg).await.unwrap_err();

    assert!(matches!(err, BootstrapError::InvalidEmail(_)), "{err}");
    assert!(lab.users.rows().is_empty());
}
