//! Tests the Users module of Scylla, written with strata: the CE application tests of the slice
//! (the user actions and the inline reset link) on stub ports, and the gRPC adapter over a real
//! tonic server.

use async_trait::async_trait;

use scylla_auth::authz::{
    Grant, GrantRepository, ORGANIZATION_ADMIN_ROLE, PROJECT_ADMIN_ROLE, PermissionService,
    Principal, SYSTEM_ADMIN_ROLE, Scope,
};
use scylla_core::application::user::reset::{RESET_LINK_TTL, ResetLinks, SendPasswordReset};
use scylla_core::application::user::{
    AccountRepository, ChangePassword, CreateUser, DeleteAccount, DeleteUser, GetMe, GetUser,
    GetUserByEmail, GetUserByUsername, ListUserAccess, ListUserSessions, PasswordResetDelivery,
    PasswordResetMessage, PasswordResetSender, RevokeUserSession, RevokeUserSessions,
    SetUserActive, UpdateUser, UpdateUserEmail, UserRepository, UserSession,
    WRONG_CURRENT_PASSWORD, WRONG_PASSWORD,
};
use scylla_core::application::{HashService, PrincipalAccess};
use scylla_core::test_support::authz::{DenyingPermissionService, RecordingPermissionService};
use scylla_core::test_support::stubs::{
    ScopesByAgent, StubAccounts, StubGrants, StubHash, StubUsers, alice, plain_hash,
};
use scylla_core::test_support::users::{UserBuilder, user};
use scylla_domain::domain::caller::CallerContext;
use scylla_domain::domain::clock;
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{AppId, OrganizationId, ProjectId, SessionId, UserId};
use scylla_domain::domain::organization::OrganizationName;
use scylla_domain::domain::permission::Permission;
use scylla_domain::domain::role::{RoleDisplayName, RoleName};
use scylla_domain::domain::session::{ACTIVITY_INTERVAL, SessionClient};
use scylla_domain::domain::user::{
    DisplayName, Email, Password, PasswordReset, ResetToken, User, Username,
};
use scylla_proto::user::v1::{GetUserRequest, ListUserSessionsRequest};
use scylla_strata::grpc::{UserServiceClient, UserServiceServer};
use scylla_strata::{
    AccessModel, Authenticator, DevTokens, Identity, ResetLinkBase, ScyllaCtx, UsersRpc,
};
use std::sync::{Arc, Mutex};
use strata::{Contextual, Edition};
use strata_tonic::Grpc;
use tonic::Code;
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Channel, Server};

const PASSWORD: &str = "SecurePass123!";
const PUBLIC_URL: &str = "https://scylla.example.com/";

/// The tokens of `DevTokens`, and `user:<id>@<session>`: a user caller with the session of the
/// call, as `ScyllaSessions` gives it.
#[derive(Default)]
pub struct SessionTokens;

impl Authenticator for SessionTokens {
    async fn authenticate(&self, token: Option<&str>) -> Identity {
        match token
            .and_then(|t| t.strip_prefix("user:"))
            .and_then(|t| t.split_once('@'))
        {
            Some((id, session)) => Identity::Caller {
                caller: CallerContext::User(UserId::new(id)),
                session: Some(SessionId::new(session)),
            },
            None => DevTokens.authenticate(token).await,
        }
    }
}

#[strata::edition(modules(scylla_strata::Access, scylla_strata::Users))]
pub struct TestBase;

#[strata::edition(extends = TestBase, overrides(scylla_strata::Access))]
pub struct TestCe;

#[strata::overrides]
impl scylla_strata::Access for TestCe {
    type Authenticator = SessionTokens;
    type Permissions = scylla_strata::CedarPermissions;
}

/// Records each message. A failing sender records nothing.
#[derive(Default)]
struct Outbox {
    sent: Mutex<Vec<PasswordResetMessage>>,
    fails: bool,
}

impl Outbox {
    fn sent(&self) -> Vec<PasswordResetMessage> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl PasswordResetSender for Outbox {
    fn delivery(&self) -> PasswordResetDelivery {
        PasswordResetDelivery::Mail
    }

    async fn send(&self, message: &PasswordResetMessage) -> DomainResult<()> {
        if self.fails {
            return Err(DomainError::infrastructure("the mail server is down"));
        }
        self.sent.lock().unwrap().push(message.clone());
        Ok(())
    }
}

struct Lab {
    app: TestCe,
    users: Arc<StubUsers>,
    accounts: Arc<StubAccounts>,
    outbox: Arc<Outbox>,
}

impl Lab {
    async fn create(&self, username: &str) -> DomainResult<User> {
        UsersRpc::create_user(&self.app.scoped(&ctx(alice())), create(username)).await
    }

    fn stored(&self, id: &UserId) -> User {
        self.users.rows()[id].clone()
    }
}

fn ctx(caller: CallerContext) -> ScyllaCtx {
    ScyllaCtx {
        caller,
        ..ScyllaCtx::default()
    }
}

async fn lab(permissions: Arc<dyn PermissionService>) -> Lab {
    lab_with(permissions, Vec::new(), Outbox::default()).await
}

async fn lab_with(
    permissions: Arc<dyn PermissionService>,
    grants: Vec<Grant>,
    outbox: Outbox,
) -> Lab {
    let users = Arc::new(StubUsers::default());
    let accounts = Arc::new(StubAccounts::new(users.clone()));
    let outbox = Arc::new(outbox);
    let config = strata::Config::new();
    let app = TestCe::builder(&config)
        .provide(AccessModel {
            permissions,
            visibility: Arc::new(ScopesByAgent::everything()),
        })
        .provide_arc::<dyn UserRepository>(users.clone())
        .provide_arc::<dyn GrantRepository>(Arc::new(StubGrants::new(grants)))
        .provide_arc::<dyn HashService>(Arc::new(StubHash::plain()))
        .provide_arc::<dyn AccountRepository>(accounts.clone())
        .provide_arc::<dyn PasswordResetSender>(outbox.clone())
        .provide(ResetLinkBase(ResetLinks::new(Some(PUBLIC_URL))))
        .build()
        .await
        .unwrap();
    Lab {
        app,
        users,
        accounts,
        outbox,
    }
}

fn create(username: &str) -> CreateUser {
    CreateUser {
        username: Username::new(username).unwrap(),
        email: email(&format!("{username}@example.com")),
        password: Password::new(PASSWORD).unwrap(),
        display_name: None,
    }
}

fn email(value: &str) -> Email {
    Email::new(value).unwrap()
}

fn password(value: &str) -> Password {
    Password::new(value).unwrap()
}

fn signed_in(id: &str) -> User {
    UserBuilder::new(id)
        .id(UserId::new(id))
        .email(format!("{id}@example.com"))
        .password_hash(plain_hash(PASSWORD).as_str())
        .build()
}

fn caller(user: &User) -> CallerContext {
    CallerContext::User(user.id().clone())
}

fn app() -> CallerContext {
    CallerContext::App(AppId::new("agent-1"))
}

fn update(user: &User) -> UpdateUser {
    UpdateUser {
        id: user.id().clone(),
        username: None,
        display_name: None,
        email: None,
    }
}

fn reset_of(user: &User) -> PasswordReset {
    PasswordReset::create(
        user.id().clone(),
        ResetToken::new(format!("{:A<43}", user.id().as_str())).unwrap(),
        RESET_LINK_TTL,
    )
}

/// Grants every permission except the ones it names.
struct Refusing(Vec<Permission>);

#[async_trait]
impl PermissionService for Refusing {
    async fn check(&self, _: &CallerContext, permission: Permission) -> DomainResult<()> {
        if self.0.contains(&permission) {
            return Err(DomainError::forbidden("denied"));
        }
        Ok(())
    }
}

#[tokio::test]
async fn a_create_checks_the_permission_then_stores_the_hashed_user() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;

    let user = lab.create("bob").await.unwrap();

    assert_eq!(permissions.permissions(), vec![Permission::CreateUser]);
    assert_eq!(lab.stored(user.id()).password_hash(), &plain_hash(PASSWORD));
    assert_eq!(user.email(), Some(&email("bob@example.com")));
}

#[tokio::test]
async fn a_create_stores_the_display_name_and_is_not_a_change() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;

    let user = UsersRpc::create_user(
        &lab.app.scoped(&ctx(alice())),
        CreateUser {
            display_name: Some(DisplayName::new("Bob B.").unwrap()),
            ..create("bob")
        },
    )
    .await
    .unwrap();

    assert_eq!(user.display_name().map(DisplayName::as_str), Some("Bob B."));
    assert_eq!(user.updated_at(), user.created_at());
}

#[tokio::test]
async fn a_denied_caller_writes_nothing() {
    let lab = lab(Arc::new(DenyingPermissionService::new())).await;

    let err = lab.create("bob").await.unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.users.rows().is_empty());
}

#[tokio::test]
async fn a_taken_username_is_a_conflict_and_writes_nothing() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    lab.create("bob").await.unwrap();

    let err = UsersRpc::create_user(
        &lab.app.scoped(&ctx(alice())),
        CreateUser {
            email: email("other@example.com"),
            ..create("bob")
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, DomainError::Conflict(_)));
    assert_eq!(lab.users.rows().len(), 1);
}

#[tokio::test]
async fn an_update_stages_the_change_and_persists_it() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let created = lab.create("old").await.unwrap();

    let updated = UsersRpc::update_user(
        &lab.app.scoped(&ctx(alice())),
        UpdateUser {
            username: Some(Username::new("new").unwrap()),
            ..update(&created)
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.username().as_str(), "new");
    assert_eq!(lab.stored(created.id()).username().as_str(), "new");
    assert_eq!(
        permissions.permissions()[1..],
        [Permission::UpdateUser(created.id().clone())]
    );
}

#[tokio::test]
async fn an_update_sets_then_removes_the_display_name() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let created = lab.create("bob").await.unwrap();
    let alice = ctx(alice());

    let named = UsersRpc::update_user(
        &lab.app.scoped(&alice),
        UpdateUser {
            display_name: Some(Some(DisplayName::new("Bobby").unwrap())),
            ..update(&created)
        },
    )
    .await
    .unwrap();
    assert_eq!(named.display_name().map(DisplayName::as_str), Some("Bobby"));

    let unnamed = UsersRpc::update_user(
        &lab.app.scoped(&alice),
        UpdateUser {
            display_name: Some(None),
            ..update(&created)
        },
    )
    .await
    .unwrap();
    assert!(unnamed.display_name().is_none());
    assert!(lab.stored(created.id()).display_name().is_none());
}

#[tokio::test]
async fn an_email_change_also_needs_create_user() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let created = lab.create("bob").await.unwrap();

    let updated = UsersRpc::update_user(
        &lab.app.scoped(&ctx(caller(&created))),
        UpdateUser {
            email: Some(email("bobby@example.com")),
            ..update(&created)
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.email(), Some(&email("bobby@example.com")));
    assert_eq!(
        permissions.permissions()[1..],
        [
            Permission::UpdateUser(created.id().clone()),
            Permission::CreateUser
        ]
    );
}

#[tokio::test]
async fn an_email_change_without_create_user_is_refused_also_on_the_own_account() {
    let lab = lab(Arc::new(Refusing(vec![Permission::CreateUser]))).await;
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());
    let as_bob = ctx(caller(&bob));

    let err = UsersRpc::update_user(
        &lab.app.scoped(&as_bob),
        UpdateUser {
            email: Some(email("bobby@example.com")),
            ..update(&bob)
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)), "{err}");
    assert_eq!(
        lab.stored(bob.id()).email(),
        Some(&email("bob@example.com"))
    );

    UsersRpc::update_user(
        &lab.app.scoped(&as_bob),
        UpdateUser {
            display_name: Some(Some(DisplayName::new("Bob").unwrap())),
            ..update(&bob)
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn an_update_to_its_own_username_is_not_a_conflict() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let created = lab.create("bob").await.unwrap();

    UsersRpc::update_user(
        &lab.app.scoped(&ctx(alice())),
        UpdateUser {
            username: Some(Username::new("bob").unwrap()),
            ..update(&created)
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn an_update_to_a_taken_username_or_email_is_a_conflict() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    lab.create("taken").await.unwrap();
    let created = lab.create("bob").await.unwrap();

    for change in [
        UpdateUser {
            username: Some(Username::new("taken").unwrap()),
            ..update(&created)
        },
        UpdateUser {
            email: Some(email("taken@example.com")),
            ..update(&created)
        },
    ] {
        let err = UsersRpc::update_user(&lab.app.scoped(&ctx(alice())), change)
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Conflict(_)));
    }
}

#[tokio::test]
async fn a_delete_removes_the_user() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let created = lab.create("gone").await.unwrap();

    UsersRpc::delete_user(
        &lab.app.scoped(&ctx(alice())),
        DeleteUser {
            id: created.id().clone(),
        },
    )
    .await
    .unwrap();

    assert!(lab.users.rows().is_empty());
    assert_eq!(
        permissions.permissions()[1],
        Permission::DeleteUser(created.id().clone())
    );
}

#[tokio::test]
async fn a_delete_of_a_missing_user_is_not_found() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;

    let err = UsersRpc::delete_user(
        &lab.app.scoped(&ctx(alice())),
        DeleteUser {
            id: UserId::new("nobody"),
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, DomainError::NotFound(_)));
}

#[tokio::test]
async fn a_read_checks_its_permission() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let created = lab.create("seen").await.unwrap();

    let read = UsersRpc::get_user(
        &lab.app.scoped(&ctx(alice())),
        GetUser {
            id: created.id().clone(),
        },
    )
    .await
    .unwrap();

    assert_eq!(read.id(), created.id());
    assert_eq!(
        permissions.permissions()[1],
        Permission::ReadUser(created.id().clone())
    );
}

#[tokio::test]
async fn get_me_reads_the_caller_without_a_permission_and_refuses_an_app() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());

    let me = UsersRpc::get_me(&lab.app.scoped(&ctx(caller(&bob))), GetMe)
        .await
        .unwrap();
    let err = UsersRpc::get_me(&lab.app.scoped(&ctx(app())), GetMe)
        .await
        .unwrap_err();

    assert_eq!(me.id(), bob.id());
    assert!(permissions.permissions().is_empty());
    assert!(matches!(err, DomainError::Forbidden(_)));
}

#[tokio::test]
async fn a_read_by_username_asks_for_list_users() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let created = lab.create("admin").await.unwrap();

    let read = UsersRpc::get_user_by_username(
        &lab.app.scoped(&ctx(alice())),
        GetUserByUsername {
            username: Username::new("admin").unwrap(),
        },
    )
    .await
    .unwrap();

    assert_eq!(read.id(), created.id());
    assert_eq!(permissions.permissions()[1], Permission::ListUsers);
}

#[tokio::test]
async fn a_read_by_email_asks_for_list_users() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let created = lab.create("admin").await.unwrap();

    let read = UsersRpc::get_user_by_email(
        &lab.app.scoped(&ctx(alice())),
        GetUserByEmail {
            email: email("admin@example.com"),
        },
    )
    .await
    .unwrap();

    assert_eq!(read.id(), created.id());
    assert_eq!(permissions.permissions()[1], Permission::ListUsers);
}

fn set_email(user: &User, value: &str) -> UpdateUserEmail {
    UpdateUserEmail {
        id: user.id().clone(),
        email: email(value),
    }
}

#[tokio::test]
async fn an_email_update_gives_an_email_to_a_user_that_has_none() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let legacy = user("admin");
    lab.users.insert(legacy.clone());

    let updated = UsersRpc::update_user_email(
        &lab.app.scoped(&ctx(alice())),
        set_email(&legacy, "admin@example.com"),
    )
    .await
    .unwrap();

    assert_eq!(updated.email(), Some(&email("admin@example.com")));
    assert_eq!(
        lab.stored(legacy.id()).email(),
        Some(&email("admin@example.com"))
    );
    assert_eq!(
        permissions.permissions(),
        vec![Permission::UpdateUser(legacy.id().clone())]
    );
}

#[tokio::test]
async fn an_email_update_keeps_another_email() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let created = lab.create("admin").await.unwrap();

    let err = UsersRpc::update_user_email(
        &lab.app.scoped(&ctx(alice())),
        set_email(&created, "second@example.com"),
    )
    .await
    .unwrap_err();

    assert!(matches!(err, DomainError::BusinessRule(_)), "{err}");
    assert_eq!(
        lab.stored(created.id()).email(),
        Some(&email("admin@example.com"))
    );
}

fn held(user: &User, role: &str, scope: Scope) -> Grant {
    Grant::new(
        Principal::User(user.id().clone()),
        RoleName::new(role).unwrap(),
        scope,
    )
}

async fn delete_with(grants: impl Fn(&User, &User) -> Vec<Grant>) -> DomainResult<()> {
    let doomed = user("doomed");
    let other = user("other");
    let lab = lab_with(
        Arc::new(RecordingPermissionService::new()),
        grants(&doomed, &other),
        Outbox::default(),
    )
    .await;
    lab.users.insert(doomed.clone());
    UsersRpc::delete_user(
        &lab.app.scoped(&ctx(alice())),
        DeleteUser {
            id: doomed.id().clone(),
        },
    )
    .await
}

fn o1() -> Scope {
    Scope::Organization(OrganizationId::new("o1"))
}

#[tokio::test]
async fn the_last_admin_of_an_organization_or_of_the_system_cannot_be_deleted() {
    for scope in [o1(), Scope::System] {
        let role = if scope == Scope::System {
            SYSTEM_ADMIN_ROLE
        } else {
            ORGANIZATION_ADMIN_ROLE
        };
        let err = delete_with(|doomed, _| vec![held(doomed, role, scope.clone())])
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::BusinessRule(_)), "{scope}");
    }
}

#[tokio::test]
async fn an_admin_is_deleted_when_another_human_admin_remains() {
    delete_with(|doomed, other| {
        vec![
            held(doomed, ORGANIZATION_ADMIN_ROLE, o1()),
            held(other, ORGANIZATION_ADMIN_ROLE, o1()),
        ]
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn the_last_project_admin_is_deleted() {
    delete_with(|doomed, _| {
        vec![held(
            doomed,
            PROJECT_ADMIN_ROLE,
            Scope::Project(ProjectId::new("p1")),
        )]
    })
    .await
    .unwrap();
}

fn change(current: &str, new: &str, session: Option<&SessionId>) -> ChangePassword {
    ChangePassword {
        current_password: password(current),
        new_password: password(new),
        session: session.cloned(),
    }
}

#[tokio::test]
async fn a_password_change_keeps_the_session_of_the_call_and_ends_the_rest() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let bob = signed_in("bob");
    let carol = signed_in("carol");
    lab.users.insert(bob.clone());
    lab.users.insert(carol.clone());
    let this = lab.accounts.open_session(bob.id());
    lab.accounts.open_session(bob.id());
    let theirs = lab.accounts.open_session(carol.id());
    lab.accounts.put_reset(reset_of(&bob));

    UsersRpc::change_password(
        &lab.app.scoped(&ctx(caller(&bob))),
        change(PASSWORD, "NewPass123!", Some(&this)),
    )
    .await
    .unwrap();

    assert_eq!(
        lab.stored(bob.id()).password_hash(),
        &plain_hash("NewPass123!")
    );
    assert_eq!(lab.accounts.sessions(bob.id()), vec![this]);
    assert_eq!(lab.accounts.sessions(carol.id()), vec![theirs]);
    assert!(lab.accounts.resets(bob.id()).is_empty());
    assert!(permissions.permissions().is_empty());
}

#[tokio::test]
async fn a_password_change_keeps_the_session_of_the_context() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());
    let this = lab.accounts.open_session(bob.id());
    lab.accounts.open_session(bob.id());
    let call = ScyllaCtx {
        session: Some(this.clone()),
        ..ctx(caller(&bob))
    };

    UsersRpc::change_password(
        &lab.app.scoped(&call),
        change(PASSWORD, "NewPass123!", None),
    )
    .await
    .unwrap();

    assert_eq!(lab.accounts.sessions(bob.id()), vec![this]);
}

#[tokio::test]
async fn a_wrong_current_password_is_a_failed_precondition_and_changes_nothing() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());
    lab.accounts.open_session(bob.id());

    let err = UsersRpc::change_password(
        &lab.app.scoped(&ctx(caller(&bob))),
        change("WrongPass123!", "NewPass123!", None),
    )
    .await
    .unwrap_err();

    assert!(matches!(&err, DomainError::BusinessRule(m) if m == WRONG_CURRENT_PASSWORD));
    assert_eq!(lab.stored(bob.id()).password_hash(), &plain_hash(PASSWORD));
    assert_eq!(lab.accounts.sessions(bob.id()).len(), 1);
}

#[tokio::test]
async fn an_app_has_no_account_to_change_or_delete() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let as_app = ctx(app());

    let change = UsersRpc::change_password(
        &lab.app.scoped(&as_app),
        change(PASSWORD, "NewPass123!", None),
    )
    .await
    .unwrap_err();
    let delete = UsersRpc::delete_account(
        &lab.app.scoped(&as_app),
        DeleteAccount {
            password: password(PASSWORD),
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(change, DomainError::Forbidden(_)));
    assert!(matches!(delete, DomainError::Forbidden(_)));
}

fn revoke(user: &User, session: Option<&SessionId>) -> RevokeUserSessions {
    RevokeUserSessions {
        id: user.id().clone(),
        session: session.cloned(),
    }
}

#[tokio::test]
async fn an_admin_revokes_every_session_of_a_user() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());
    lab.accounts.open_session(bob.id());
    lab.accounts.open_session(bob.id());
    let admins = lab.accounts.open_session(&UserId::new("alice"));

    let revoked =
        UsersRpc::revoke_user_sessions(&lab.app.scoped(&ctx(alice())), revoke(&bob, Some(&admins)))
            .await
            .unwrap();

    assert_eq!(revoked, 2);
    assert!(lab.accounts.sessions(bob.id()).is_empty());
    assert_eq!(lab.accounts.sessions(&UserId::new("alice")), vec![admins]);
    assert_eq!(
        permissions.permissions(),
        vec![Permission::UpdateUser(bob.id().clone())]
    );
}

#[tokio::test]
async fn a_user_that_revokes_its_own_sessions_keeps_the_session_of_the_call() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());
    let this = lab.accounts.open_session(bob.id());
    lab.accounts.open_session(bob.id());
    lab.accounts.open_session(bob.id());

    let revoked = UsersRpc::revoke_user_sessions(
        &lab.app.scoped(&ctx(caller(&bob))),
        revoke(&bob, Some(&this)),
    )
    .await
    .unwrap();

    assert_eq!(revoked, 2);
    assert_eq!(lab.accounts.sessions(bob.id()), vec![this]);
}

#[tokio::test]
async fn a_revoke_for_an_unknown_user_is_not_found() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;

    let err = UsersRpc::revoke_user_sessions(
        &lab.app.scoped(&ctx(alice())),
        revoke(&user("ghost"), None),
    )
    .await
    .unwrap_err();

    assert!(err.is_not_found());
}

fn set_active(user: &User, is_active: bool) -> SetUserActive {
    SetUserActive {
        id: user.id().clone(),
        is_active,
    }
}

#[tokio::test]
async fn a_deactivation_ends_the_sessions_and_the_reset_links() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());
    lab.accounts.open_session(bob.id());
    lab.accounts.put_reset(reset_of(&bob));

    let off = UsersRpc::set_user_active(&lab.app.scoped(&ctx(alice())), set_active(&bob, false))
        .await
        .unwrap();

    assert!(!off.is_active());
    assert!(!lab.stored(bob.id()).is_active());
    assert!(lab.accounts.sessions(bob.id()).is_empty());
    assert!(lab.accounts.resets(bob.id()).is_empty());
    assert_eq!(
        permissions.permissions(),
        vec![Permission::UpdateUser(bob.id().clone())]
    );

    let on = UsersRpc::set_user_active(&lab.app.scoped(&ctx(alice())), set_active(&bob, true))
        .await
        .unwrap();
    assert!(on.is_active());
    assert!(lab.stored(bob.id()).is_active());
}

#[tokio::test]
async fn setting_the_current_active_value_writes_nothing() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());
    let session = lab.accounts.open_session(bob.id());

    let same = UsersRpc::set_user_active(&lab.app.scoped(&ctx(alice())), set_active(&bob, true))
        .await
        .unwrap();

    assert!(same.is_active());
    assert_eq!(same.updated_at(), bob.updated_at());
    assert_eq!(lab.accounts.sessions(bob.id()), vec![session]);
}

#[tokio::test]
async fn a_user_cannot_change_its_own_active_flag() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());

    for is_active in [false, true] {
        let err = UsersRpc::set_user_active(
            &lab.app.scoped(&ctx(caller(&bob))),
            set_active(&bob, is_active),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DomainError::BusinessRule(_)), "{err}");
    }
    assert!(lab.stored(bob.id()).is_active());
}

#[tokio::test]
async fn the_last_active_system_admin_stays_active() {
    let root = signed_in("root");
    let other = signed_in("other");
    let idle = UserBuilder::new("idle")
        .id(UserId::new("idle"))
        .is_active(false)
        .build();
    for (admins, allowed) in [
        (vec![&root], false),
        (vec![&root, &idle], false),
        (vec![&root, &other], true),
    ] {
        let grants = admins
            .iter()
            .map(|u| held(u, SYSTEM_ADMIN_ROLE, Scope::System))
            .collect();
        let lab = lab_with(
            Arc::new(RecordingPermissionService::new()),
            grants,
            Outbox::default(),
        )
        .await;
        for user in [&root, &other, &idle] {
            lab.users.insert((*user).clone());
        }

        let result =
            UsersRpc::set_user_active(&lab.app.scoped(&ctx(alice())), set_active(&root, false))
                .await;

        let ids: Vec<&UserId> = admins.iter().map(|u| u.id()).collect();
        assert_eq!(result.is_ok(), allowed, "{ids:?}");
        assert!(allowed || matches!(result, Err(DomainError::BusinessRule(_))));
        assert_eq!(lab.stored(root.id()).is_active(), !allowed);
    }
}

#[tokio::test]
async fn a_user_that_is_no_system_admin_is_deactivated_without_the_admin_rule() {
    let root = signed_in("root");
    let bob = signed_in("bob");
    let lab = lab_with(
        Arc::new(RecordingPermissionService::new()),
        vec![held(&root, SYSTEM_ADMIN_ROLE, Scope::System)],
        Outbox::default(),
    )
    .await;
    lab.users.insert(root);
    lab.users.insert(bob.clone());

    UsersRpc::set_user_active(&lab.app.scoped(&ctx(alice())), set_active(&bob, false))
        .await
        .unwrap();
}

fn delete_account(value: &str) -> DeleteAccount {
    DeleteAccount {
        password: password(value),
    }
}

#[tokio::test]
async fn an_account_is_deleted_with_the_right_password() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());
    let as_bob = ctx(caller(&bob));

    let wrong = UsersRpc::delete_account(&lab.app.scoped(&as_bob), delete_account("WrongPass123!"))
        .await
        .unwrap_err();
    assert!(matches!(&wrong, DomainError::BusinessRule(m) if m == WRONG_PASSWORD));
    assert!(lab.users.rows().contains_key(bob.id()));

    UsersRpc::delete_account(&lab.app.scoped(&as_bob), delete_account(PASSWORD))
        .await
        .unwrap();
    assert!(lab.users.rows().is_empty());
    assert!(permissions.permissions().is_empty());
}

fn named(user: &User, organization: &str, name: &str) -> PrincipalAccess {
    PrincipalAccess {
        grant_id: format!("{}-{organization}", user.id()),
        scope: Scope::Organization(OrganizationId::new(organization)),
        organization_id: Some(OrganizationId::new(organization)),
        organization_name: Some(OrganizationName::new(name).unwrap()),
        project_name: None,
        role: RoleName::new(ORGANIZATION_ADMIN_ROLE).unwrap(),
        role_name: RoleDisplayName::new("Organization admin").unwrap(),
    }
}

#[tokio::test]
async fn the_last_admin_cannot_delete_its_account_and_the_message_names_the_organizations() {
    let bob = signed_in("bob");
    let carol = signed_in("carol");
    let org = |id: &str| Scope::Organization(OrganizationId::new(id));
    let lab = lab_with(
        Arc::new(RecordingPermissionService::new()),
        vec![
            held(&bob, ORGANIZATION_ADMIN_ROLE, org("o1")),
            held(&bob, ORGANIZATION_ADMIN_ROLE, org("o2")),
            held(&bob, ORGANIZATION_ADMIN_ROLE, org("o3")),
            held(&carol, ORGANIZATION_ADMIN_ROLE, org("o3")),
            held(&bob, SYSTEM_ADMIN_ROLE, Scope::System),
        ],
        Outbox::default(),
    )
    .await;
    lab.users.insert(bob.clone());
    lab.accounts
        .grant_access(bob.id(), named(&bob, "o1", "Zeta"));
    lab.accounts
        .grant_access(bob.id(), named(&bob, "o2", "Acme"));
    lab.accounts
        .grant_access(bob.id(), named(&bob, "o3", "Shared"));

    let err = UsersRpc::delete_account(
        &lab.app.scoped(&ctx(caller(&bob))),
        delete_account(PASSWORD),
    )
    .await
    .unwrap_err();

    let DomainError::BusinessRule(message) = err else {
        panic!("expected a business rule, got {err}");
    };
    assert_eq!(
        message,
        "Cannot delete your account: you are the last system administrator and the last \
         organization administrator of Acme, Zeta. Appoint another administrator first."
    );
    assert!(lab.users.rows().contains_key(bob.id()));
}

#[tokio::test]
async fn an_admin_that_shares_each_organization_deletes_its_account() {
    let bob = signed_in("bob");
    let carol = signed_in("carol");
    let lab = lab_with(
        Arc::new(RecordingPermissionService::new()),
        vec![
            held(&bob, ORGANIZATION_ADMIN_ROLE, o1()),
            held(&carol, ORGANIZATION_ADMIN_ROLE, o1()),
            held(
                &bob,
                PROJECT_ADMIN_ROLE,
                Scope::Project(ProjectId::new("p1")),
            ),
        ],
        Outbox::default(),
    )
    .await;
    lab.users.insert(bob.clone());

    UsersRpc::delete_account(
        &lab.app.scoped(&ctx(caller(&bob))),
        delete_account(PASSWORD),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn the_access_list_needs_read_user_and_a_known_user() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());
    lab.accounts
        .grant_access(bob.id(), named(&bob, "o1", "Acme"));
    let alice = ctx(alice());

    let access = UsersRpc::list_user_access(
        &lab.app.scoped(&alice),
        ListUserAccess {
            id: bob.id().clone(),
        },
    )
    .await
    .unwrap();
    let unknown = UsersRpc::list_user_access(
        &lab.app.scoped(&alice),
        ListUserAccess {
            id: UserId::new("ghost"),
        },
    )
    .await
    .unwrap_err();

    assert_eq!(access, vec![named(&bob, "o1", "Acme")]);
    assert_eq!(
        permissions.permissions()[0],
        Permission::ReadUser(bob.id().clone())
    );
    assert!(unknown.is_not_found());
}

fn session_of(age: i32, expires_in: i32) -> UserSession {
    let minute = ACTIVITY_INTERVAL / 5;
    let now = clock::now();
    UserSession {
        id: SessionId::generate(),
        created_at: now - minute * (age + 1),
        last_active_at: now - minute * age,
        expires_at: now + minute * expires_in,
        client: SessionClient::default(),
        current: false,
    }
}

fn put(lab: &Lab, user: &User, session: &UserSession) -> SessionId {
    lab.accounts.put_session(user.id(), session.clone());
    session.id.clone()
}

fn list(user: &User, session: Option<&SessionId>) -> ListUserSessions {
    ListUserSessions {
        id: user.id().clone(),
        session: session.cloned(),
    }
}

fn revoke_one(user: &User, session: &SessionId) -> RevokeUserSession {
    RevokeUserSession {
        id: user.id().clone(),
        session_id: session.clone(),
    }
}

#[tokio::test]
async fn a_list_shows_the_live_sessions_most_recently_active_first_and_marks_the_current_one() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let bob = signed_in("bob");
    let carol = signed_in("carol");
    lab.users.insert(bob.clone());
    lab.users.insert(carol.clone());
    let hour = 60;
    let old = put(&lab, &bob, &session_of(60 * 2, hour));
    let new = put(&lab, &bob, &session_of(1, hour));
    let this = put(&lab, &bob, &session_of(30, hour));
    put(&lab, &bob, &session_of(60 * 3, -1));
    put(&lab, &carol, &session_of(0, hour));

    let sessions =
        UsersRpc::list_user_sessions(&lab.app.scoped(&ctx(caller(&bob))), list(&bob, Some(&this)))
            .await
            .unwrap();

    let ids: Vec<_> = sessions.iter().map(|s| s.id.clone()).collect();
    assert_eq!(ids, vec![new, this.clone(), old]);
    let current: Vec<_> = sessions
        .iter()
        .filter(|s| s.current)
        .map(|s| &s.id)
        .collect();
    assert_eq!(current, vec![&this]);
    assert_eq!(
        permissions.permissions(),
        vec![Permission::ReadUser(bob.id().clone())]
    );
}

#[tokio::test]
async fn the_sessions_of_another_user_need_the_read_permission() {
    let bob = signed_in("bob");
    let carol = signed_in("carol");
    let refused = lab(Arc::new(Refusing(vec![Permission::ReadUser(
        bob.id().clone(),
    )])))
    .await;
    let allowed = lab(Arc::new(Refusing(Vec::new()))).await;
    for lab in [&refused, &allowed] {
        lab.users.insert(bob.clone());
        lab.users.insert(carol.clone());
        put(lab, &bob, &session_of(0, 60));
    }
    let carols = allowed.accounts.open_session(carol.id());
    let as_carol = ctx(caller(&carol));

    let err = UsersRpc::list_user_sessions(&refused.app.scoped(&as_carol), list(&bob, None))
        .await
        .unwrap_err();
    let seen =
        UsersRpc::list_user_sessions(&allowed.app.scoped(&as_carol), list(&bob, Some(&carols)))
            .await
            .unwrap();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(seen.len(), 1);
    assert!(!seen[0].current);
}

#[tokio::test]
async fn a_session_list_for_an_unknown_user_is_not_found() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;

    let err =
        UsersRpc::list_user_sessions(&lab.app.scoped(&ctx(alice())), list(&user("ghost"), None))
            .await
            .unwrap_err();

    assert!(err.is_not_found());
}

#[tokio::test]
async fn a_user_revokes_one_of_its_sessions_and_can_revoke_the_session_of_the_call() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());
    let this = lab.accounts.open_session(bob.id());
    let other = lab.accounts.open_session(bob.id());
    let as_bob = ScyllaCtx {
        session: Some(this.clone()),
        ..ctx(caller(&bob))
    };

    UsersRpc::revoke_user_session(&lab.app.scoped(&as_bob), revoke_one(&bob, &other))
        .await
        .unwrap();
    assert_eq!(lab.accounts.sessions(bob.id()), vec![this.clone()]);

    UsersRpc::revoke_user_session(&lab.app.scoped(&as_bob), revoke_one(&bob, &this))
        .await
        .unwrap();
    assert!(lab.accounts.sessions(bob.id()).is_empty());
    assert_eq!(
        permissions.permissions(),
        vec![
            Permission::UpdateUser(bob.id().clone()),
            Permission::UpdateUser(bob.id().clone())
        ]
    );
}

#[tokio::test]
async fn a_session_of_another_user_needs_the_update_permission() {
    let bob = signed_in("bob");
    let carol = signed_in("carol");
    let refused = lab(Arc::new(Refusing(vec![Permission::UpdateUser(
        bob.id().clone(),
    )])))
    .await;
    let allowed = lab(Arc::new(Refusing(Vec::new()))).await;
    let mut ids = Vec::new();
    for lab in [&refused, &allowed] {
        lab.users.insert(bob.clone());
        lab.users.insert(carol.clone());
        ids.push(lab.accounts.open_session(bob.id()));
    }
    let as_carol = ctx(caller(&carol));

    let err =
        UsersRpc::revoke_user_session(&refused.app.scoped(&as_carol), revoke_one(&bob, &ids[0]))
            .await
            .unwrap_err();
    UsersRpc::revoke_user_session(&allowed.app.scoped(&as_carol), revoke_one(&bob, &ids[1]))
        .await
        .unwrap();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(refused.accounts.sessions(bob.id()), vec![ids[0].clone()]);
    assert!(allowed.accounts.sessions(bob.id()).is_empty());
}

#[tokio::test]
async fn a_session_that_is_not_a_live_session_of_the_user_gives_one_not_found() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let bob = signed_in("bob");
    let carol = signed_in("carol");
    lab.users.insert(bob.clone());
    lab.users.insert(carol.clone());
    let bobs = lab.accounts.open_session(bob.id());
    let expired = put(&lab, &carol, &session_of(60 * 2, -1));
    let as_carol = ctx(caller(&carol));

    for (owner, id) in [
        (&carol, bobs.clone()),
        (&carol, expired.clone()),
        (&carol, SessionId::new("unknown")),
        (&user("ghost"), bobs.clone()),
    ] {
        let err = UsersRpc::revoke_user_session(&lab.app.scoped(&as_carol), revoke_one(owner, &id))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, DomainError::NotFound(m) if m == &format!("Session '{id}' not found"))
        );
    }
    assert_eq!(lab.accounts.sessions(bob.id()), vec![bobs]);
    assert_eq!(lab.accounts.sessions(carol.id()), vec![expired]);
}

fn account(lab: &Lab, id: &str, active: bool) -> User {
    let user = UserBuilder::new(id)
        .id(UserId::new(id))
        .email(format!("{id}@example.com"))
        .display_name("Kevin K.")
        .password_hash(plain_hash(PASSWORD).as_str())
        .is_active(active)
        .build();
    lab.users.insert(user.clone());
    user
}

fn token_of(message: &PasswordResetMessage) -> ResetToken {
    let token = message.link.rsplit_once("#token=").unwrap().1;
    ResetToken::new(token).unwrap()
}

async fn send(lab: &Lab, user: &User) -> DomainResult<PasswordResetDelivery> {
    UsersRpc::send_password_reset(
        &lab.app.scoped(&ctx(alice())),
        SendPasswordReset {
            id: user.id().clone(),
        },
    )
    .await
}

#[tokio::test]
async fn an_administrator_sends_a_link_inline_without_the_minute_rule() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let kevin = account(&lab, "kevin", true);

    let first = send(&lab, &kevin).await.unwrap();
    assert_eq!(lab.outbox.sent().len(), 1);
    send(&lab, &kevin).await.unwrap();

    assert_eq!(first, PasswordResetDelivery::Mail);
    assert_eq!(lab.outbox.sent().len(), 2);
    assert!(
        lab.outbox.sent()[1]
            .link
            .starts_with("https://scylla.example.com/reset-password#token=")
    );
    let resets = lab.accounts.resets(kevin.id());
    assert_eq!(resets.len(), 1);
    assert_eq!(token_of(&lab.outbox.sent()[1]), *resets[0].token());
    assert_eq!(
        permissions.permissions(),
        vec![
            Permission::UpdateUser(kevin.id().clone()),
            Permission::UpdateUser(kevin.id().clone())
        ]
    );
}

#[tokio::test]
async fn a_link_for_an_inactive_account_or_an_account_without_an_email_is_refused() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let idle = account(&lab, "idle", false);
    let legacy = UserBuilder::new("legacy").id(UserId::new("legacy")).build();
    lab.users.insert(legacy.clone());

    for user in [&idle, &legacy] {
        let err = send(&lab, user).await.unwrap_err();
        assert!(matches!(err, DomainError::BusinessRule(_)), "{err}");
    }
    assert!(lab.outbox.sent().is_empty());
}

#[tokio::test]
async fn a_failed_send_is_the_error_of_the_administrator() {
    let lab = lab_with(
        Arc::new(RecordingPermissionService::new()),
        Vec::new(),
        Outbox {
            fails: true,
            ..Outbox::default()
        },
    )
    .await;
    let kevin = account(&lab, "kevin", true);

    let err = send(&lab, &kevin).await.unwrap_err();

    assert!(matches!(err, DomainError::Infrastructure(_)), "{err}");
}

/// Grants each permission to each caller except `user:eve`.
struct AllButEve;

#[async_trait]
impl PermissionService for AllButEve {
    async fn check(&self, caller: &CallerContext, permission: Permission) -> DomainResult<()> {
        match caller {
            CallerContext::User(id) if id.as_str() == "eve" => Err(DomainError::forbidden(
                format!("{} is not granted", permission.key()),
            )),
            _ => Ok(()),
        }
    }
}

async fn serve(lab: Lab) -> UserServiceClient<Channel> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Server::builder().add_service(UserServiceServer::new(Grpc::new(lab.app)));
    tokio::spawn(router.serve_with_incoming(TcpIncoming::from(listener)));
    let channel = Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    UserServiceClient::new(channel)
}

fn bearer<T>(message: T, bearer: Option<&str>) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    if let Some(bearer) = bearer {
        request
            .metadata_mut()
            .insert("authorization", bearer.parse().unwrap());
    }
    request
}

fn of(id: &str) -> Option<scylla_proto::common::v1::UserId> {
    Some(scylla_proto::common::v1::UserId {
        value: id.to_owned(),
    })
}

#[tokio::test]
async fn grpc_serves_the_scylla_contract() {
    let lab = lab(Arc::new(AllButEve)).await;
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());
    let this = lab.accounts.open_session(bob.id());
    lab.accounts.open_session(bob.id());
    let mut client = serve(lab).await;

    let read = client
        .get_user(bearer(
            GetUserRequest { user_id: of("bob") },
            Some("Bearer user:ann"),
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(read.user.unwrap().username, "bob");

    let anonymous = client
        .get_user(bearer(GetUserRequest { user_id: of("bob") }, None))
        .await
        .unwrap_err();
    assert_eq!(anonymous.code(), Code::Unauthenticated, "{anonymous:?}");
    let denied = client
        .get_user(bearer(
            GetUserRequest { user_id: of("bob") },
            Some("Bearer user:eve"),
        ))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::PermissionDenied, "{denied:?}");

    let token = format!("Bearer user:bob@{this}");
    let sessions = client
        .list_user_sessions(bearer(
            ListUserSessionsRequest { user_id: of("bob") },
            Some(&token),
        ))
        .await
        .unwrap()
        .into_inner()
        .sessions;
    assert_eq!(sessions.len(), 2);
    assert_eq!(
        sessions.iter().filter(|s| s.current).count(),
        1,
        "the session of the call is current"
    );
}
