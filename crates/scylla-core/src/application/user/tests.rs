//! The user's actions through the engine, on stub ports.

use super::*;
use crate::application::PrincipalAccess;
use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::domain::caller::CallerContext;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{AppId, OrganizationId, ProjectId, SessionId, UserId};
use crate::domain::organization::OrganizationName;
use crate::domain::permission::Permission;
use crate::domain::role::{RoleDisplayName, RoleName};
use crate::domain::session::SessionClient;
use crate::domain::user::{
    DisplayName, Email, Password, PasswordReset, ResetToken, User, Username,
};
use crate::test_support::authz::{DenyingPermissionService, RecordingPermissionService, actions};
use crate::test_support::stubs::{
    StubAccounts, StubGrants, StubHash, StubUsers, alice, plain_hash,
};
use crate::test_support::users::{UserBuilder, user};
use async_trait::async_trait;
use chrono::Duration;
use scylla_auth::authz::{
    Grant, ORGANIZATION_ADMIN_ROLE, PROJECT_ADMIN_ROLE, PermissionService, Principal,
    SYSTEM_ADMIN_ROLE, Scope,
};
use scylla_extension::Actions;

const PASSWORD: &str = "SecurePass123!";

struct Lab {
    actions: Actions,
    uc: UserUseCases,
    users: Arc<StubUsers>,
    accounts: Arc<StubAccounts>,
}

impl Lab {
    async fn create(&self, username: &str) -> DomainResult<User> {
        self.actions.run(&self.uc, &alice(), create(username)).await
    }

    fn stored(&self, id: &UserId) -> User {
        self.users.rows()[id].clone()
    }
}

fn lab(permissions: Arc<dyn PermissionService>) -> Lab {
    lab_with(permissions, Vec::new())
}

fn lab_with(permissions: Arc<dyn PermissionService>, grants: Vec<Grant>) -> Lab {
    let users = Arc::new(StubUsers::default());
    let accounts = Arc::new(StubAccounts::new(users.clone()));
    Lab {
        actions: actions(permissions),
        uc: UserUseCases::new(
            users.clone(),
            Arc::new(StubGrants::new(grants)),
            Arc::new(StubHash::plain()),
            accounts.clone(),
        ),
        users,
        accounts,
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
        Duration::hours(1),
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
    let lab = lab(permissions.clone());

    let user = lab.create("bob").await.unwrap();

    assert_eq!(permissions.permissions(), vec![Permission::CreateUser]);
    assert_eq!(lab.stored(user.id()).password_hash(), &plain_hash(PASSWORD));
    assert_eq!(user.email(), Some(&email("bob@example.com")));
}

#[tokio::test]
async fn a_create_stores_the_display_name_and_is_not_a_change() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));

    let user = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
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
    let lab = lab(Arc::new(DenyingPermissionService::new()));

    let err = lab.create("bob").await.unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.users.rows().is_empty());
}

#[tokio::test]
async fn a_taken_username_is_a_conflict_and_writes_nothing() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    lab.create("bob").await.unwrap();

    let err = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
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
    let lab = lab(permissions.clone());
    let created = lab.create("old").await.unwrap();

    let updated = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
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
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let created = lab.create("bob").await.unwrap();

    let named = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            UpdateUser {
                display_name: Some(Some(DisplayName::new("Bobby").unwrap())),
                ..update(&created)
            },
        )
        .await
        .unwrap();
    assert_eq!(named.display_name().map(DisplayName::as_str), Some("Bobby"));

    let unnamed = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
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
    let lab = lab(permissions.clone());
    let created = lab.create("bob").await.unwrap();

    let updated = lab
        .actions
        .run(
            &lab.uc,
            &caller(&created),
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
    let lab = lab(Arc::new(Refusing(vec![Permission::CreateUser])));
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());

    let err = lab
        .actions
        .run(
            &lab.uc,
            &caller(&bob),
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

    lab.actions
        .run(
            &lab.uc,
            &caller(&bob),
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
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let created = lab.create("bob").await.unwrap();

    lab.actions
        .run(
            &lab.uc,
            &alice(),
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
    let lab = lab(Arc::new(RecordingPermissionService::new()));
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
        let err = lab
            .actions
            .run(&lab.uc, &alice(), change)
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Conflict(_)));
    }
}

#[tokio::test]
async fn a_delete_returns_the_tombstone() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let created = lab.create("gone").await.unwrap();

    let deleted = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            DeleteUser {
                id: created.id().clone(),
            },
        )
        .await
        .unwrap();

    assert_eq!(deleted.last_state().id(), created.id());
    assert!(lab.users.rows().is_empty());
    assert_eq!(
        permissions.permissions()[1],
        Permission::DeleteUser(created.id().clone())
    );
}

#[tokio::test]
async fn a_delete_of_a_missing_user_is_not_found() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));

    let err = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
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
    let lab = lab(permissions.clone());
    let created = lab.create("seen").await.unwrap();

    let read = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
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
    let lab = lab(permissions.clone());
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());

    let me = lab
        .actions
        .run(&lab.uc, &caller(&bob), GetMe)
        .await
        .unwrap();
    let err = lab.actions.run(&lab.uc, &app(), GetMe).await.unwrap_err();

    assert_eq!(me.id(), bob.id());
    assert!(permissions.permissions().is_empty());
    assert!(matches!(err, DomainError::Forbidden(_)));
}

#[tokio::test]
async fn a_read_by_username_asks_for_list_users() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let created = lab.create("admin").await.unwrap();

    let read = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
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
    let lab = lab(permissions.clone());
    let created = lab.create("admin").await.unwrap();

    let read = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
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
    let lab = lab(permissions.clone());
    let legacy = user("admin");
    lab.users.insert(legacy.clone());

    let updated = lab
        .actions
        .run(&lab.uc, &alice(), set_email(&legacy, "admin@example.com"))
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
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let created = lab.create("admin").await.unwrap();

    let err = lab
        .actions
        .run(&lab.uc, &alice(), set_email(&created, "second@example.com"))
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::BusinessRule(_)), "{err}");
    assert_eq!(
        lab.stored(created.id()).email(),
        Some(&email("admin@example.com"))
    );
}

#[tokio::test]
async fn users_in_order_keeps_the_page_order_and_metadata_and_drops_unknown_ids() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let bob = lab.create("bob").await.unwrap();
    let carol = lab.create("carol").await.unwrap();
    let ids = vec![bob.id().clone(), UserId::new("ghost"), carol.id().clone()];
    let page = PaginatedResult::new(ids, &PaginationParams::default(), 7);

    let users = users_in_order(lab.users.as_ref(), page).await.unwrap();

    let got: Vec<&UserId> = users.items().iter().map(User::id).collect();
    assert_eq!(got, vec![bob.id(), carol.id()]);
    assert_eq!(users.metadata().total_count(), 7);
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
    );
    lab.users.insert(doomed.clone());
    lab.actions
        .run(
            &lab.uc,
            &alice(),
            DeleteUser {
                id: doomed.id().clone(),
            },
        )
        .await
        .map(|_| ())
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
    let lab = lab(permissions.clone());
    let bob = signed_in("bob");
    let carol = signed_in("carol");
    lab.users.insert(bob.clone());
    lab.users.insert(carol.clone());
    let this = lab.accounts.open_session(bob.id());
    lab.accounts.open_session(bob.id());
    let theirs = lab.accounts.open_session(carol.id());
    lab.accounts.put_reset(reset_of(&bob));

    lab.actions
        .run(
            &lab.uc,
            &caller(&bob),
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
async fn a_wrong_current_password_is_a_failed_precondition_and_changes_nothing() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());
    lab.accounts.open_session(bob.id());

    let err = lab
        .actions
        .run(
            &lab.uc,
            &caller(&bob),
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
    let lab = lab(Arc::new(RecordingPermissionService::new()));

    let change = lab
        .actions
        .run(&lab.uc, &app(), change(PASSWORD, "NewPass123!", None))
        .await
        .unwrap_err();
    let delete = lab
        .actions
        .run(
            &lab.uc,
            &app(),
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
    let lab = lab(permissions.clone());
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());
    lab.accounts.open_session(bob.id());
    lab.accounts.open_session(bob.id());
    let admins = lab.accounts.open_session(&UserId::new("alice"));

    let revoked = lab
        .actions
        .run(&lab.uc, &alice(), revoke(&bob, Some(&admins)))
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
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());
    let this = lab.accounts.open_session(bob.id());
    lab.accounts.open_session(bob.id());
    lab.accounts.open_session(bob.id());

    let revoked = lab
        .actions
        .run(&lab.uc, &caller(&bob), revoke(&bob, Some(&this)))
        .await
        .unwrap();

    assert_eq!(revoked, 2);
    assert_eq!(lab.accounts.sessions(bob.id()), vec![this]);
}

#[tokio::test]
async fn a_revoke_for_an_unknown_user_is_not_found() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));

    let err = lab
        .actions
        .run(&lab.uc, &alice(), revoke(&user("ghost"), None))
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
    let lab = lab(permissions.clone());
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());
    lab.accounts.open_session(bob.id());
    lab.accounts.put_reset(reset_of(&bob));

    let off = lab
        .actions
        .run(&lab.uc, &alice(), set_active(&bob, false))
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

    let on = lab
        .actions
        .run(&lab.uc, &alice(), set_active(&bob, true))
        .await
        .unwrap();
    assert!(on.is_active());
    assert!(lab.stored(bob.id()).is_active());
}

#[tokio::test]
async fn setting_the_current_active_value_writes_nothing() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());
    let session = lab.accounts.open_session(bob.id());

    let same = lab
        .actions
        .run(&lab.uc, &alice(), set_active(&bob, true))
        .await
        .unwrap();

    assert!(same.is_active());
    assert_eq!(same.updated_at(), bob.updated_at());
    assert_eq!(lab.accounts.sessions(bob.id()), vec![session]);
}

#[tokio::test]
async fn a_user_cannot_change_its_own_active_flag() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());

    for is_active in [false, true] {
        let err = lab
            .actions
            .run(&lab.uc, &caller(&bob), set_active(&bob, is_active))
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
        let lab = lab_with(Arc::new(RecordingPermissionService::new()), grants);
        for user in [&root, &other, &idle] {
            lab.users.insert((*user).clone());
        }

        let result = lab
            .actions
            .run(&lab.uc, &alice(), set_active(&root, false))
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
    );
    lab.users.insert(root);
    lab.users.insert(bob.clone());

    lab.actions
        .run(&lab.uc, &alice(), set_active(&bob, false))
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
    let lab = lab(permissions.clone());
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());

    let wrong = lab
        .actions
        .run(&lab.uc, &caller(&bob), delete_account("WrongPass123!"))
        .await
        .unwrap_err();
    assert!(matches!(&wrong, DomainError::BusinessRule(m) if m == WRONG_PASSWORD));
    assert!(lab.users.rows().contains_key(bob.id()));

    let deleted = lab
        .actions
        .run(&lab.uc, &caller(&bob), delete_account(PASSWORD))
        .await
        .unwrap();
    assert_eq!(deleted.last_state().id(), bob.id());
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
    );
    lab.users.insert(bob.clone());
    lab.accounts
        .grant_access(bob.id(), named(&bob, "o1", "Zeta"));
    lab.accounts
        .grant_access(bob.id(), named(&bob, "o2", "Acme"));
    lab.accounts
        .grant_access(bob.id(), named(&bob, "o3", "Shared"));

    let err = lab
        .actions
        .run(&lab.uc, &caller(&bob), delete_account(PASSWORD))
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
    );
    lab.users.insert(bob.clone());

    lab.actions
        .run(&lab.uc, &caller(&bob), delete_account(PASSWORD))
        .await
        .unwrap();
}

#[tokio::test]
async fn the_access_list_needs_read_user_and_a_known_user() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());
    lab.accounts
        .grant_access(bob.id(), named(&bob, "o1", "Acme"));

    let access = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            ListUserAccess {
                id: bob.id().clone(),
            },
        )
        .await
        .unwrap();
    let unknown = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
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

fn session_of(age: Duration, expires_in: Duration) -> UserSession {
    let now = crate::domain::clock::now();
    UserSession {
        id: SessionId::generate(),
        created_at: now - age - Duration::minutes(1),
        last_active_at: now - age,
        expires_at: now + expires_in,
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
    let lab = lab(permissions.clone());
    let bob = signed_in("bob");
    let carol = signed_in("carol");
    lab.users.insert(bob.clone());
    lab.users.insert(carol.clone());
    let hour = Duration::hours(1);
    let old = put(&lab, &bob, &session_of(Duration::hours(2), hour));
    let new = put(&lab, &bob, &session_of(Duration::minutes(1), hour));
    let this = put(&lab, &bob, &session_of(Duration::minutes(30), hour));
    put(
        &lab,
        &bob,
        &session_of(Duration::hours(3), -Duration::minutes(1)),
    );
    put(&lab, &carol, &session_of(Duration::zero(), hour));

    let sessions = lab
        .actions
        .run(&lab.uc, &caller(&bob), list(&bob, Some(&this)))
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
    )])));
    let allowed = lab(Arc::new(Refusing(Vec::new())));
    for lab in [&refused, &allowed] {
        lab.users.insert(bob.clone());
        lab.users.insert(carol.clone());
        put(lab, &bob, &session_of(Duration::zero(), Duration::hours(1)));
    }
    let carols = allowed.accounts.open_session(carol.id());

    let err = refused
        .actions
        .run(&refused.uc, &caller(&carol), list(&bob, None))
        .await
        .unwrap_err();
    let seen = allowed
        .actions
        .run(&allowed.uc, &caller(&carol), list(&bob, Some(&carols)))
        .await
        .unwrap();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(seen.len(), 1);
    assert!(!seen[0].current);
}

#[tokio::test]
async fn a_session_list_for_an_unknown_user_is_not_found() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));

    let err = lab
        .actions
        .run(&lab.uc, &alice(), list(&user("ghost"), None))
        .await
        .unwrap_err();

    assert!(err.is_not_found());
}

#[tokio::test]
async fn a_user_revokes_one_of_its_sessions_and_can_revoke_the_session_of_the_call() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let bob = signed_in("bob");
    lab.users.insert(bob.clone());
    let this = lab.accounts.open_session(bob.id());
    let other = lab.accounts.open_session(bob.id());

    let revoked = lab
        .actions
        .run(&lab.uc, &caller(&bob), revoke_one(&bob, &other))
        .await
        .unwrap();
    assert_eq!(revoked.last_state().id, other);
    assert_eq!(lab.accounts.sessions(bob.id()), vec![this.clone()]);

    lab.actions
        .run(&lab.uc, &caller(&bob), revoke_one(&bob, &this))
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
    )])));
    let allowed = lab(Arc::new(Refusing(Vec::new())));
    let mut ids = Vec::new();
    for lab in [&refused, &allowed] {
        lab.users.insert(bob.clone());
        lab.users.insert(carol.clone());
        ids.push(lab.accounts.open_session(bob.id()));
    }

    let err = refused
        .actions
        .run(&refused.uc, &caller(&carol), revoke_one(&bob, &ids[0]))
        .await
        .unwrap_err();
    allowed
        .actions
        .run(&allowed.uc, &caller(&carol), revoke_one(&bob, &ids[1]))
        .await
        .unwrap();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(refused.accounts.sessions(bob.id()), vec![ids[0].clone()]);
    assert!(allowed.accounts.sessions(bob.id()).is_empty());
}

#[tokio::test]
async fn a_session_that_is_not_a_live_session_of_the_user_gives_one_not_found() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let bob = signed_in("bob");
    let carol = signed_in("carol");
    lab.users.insert(bob.clone());
    lab.users.insert(carol.clone());
    let bobs = lab.accounts.open_session(bob.id());
    let expired = put(
        &lab,
        &carol,
        &session_of(Duration::hours(2), -Duration::minutes(1)),
    );

    for (owner, id) in [
        (&carol, bobs.clone()),
        (&carol, expired.clone()),
        (&carol, SessionId::new("unknown")),
        (&user("ghost"), bobs.clone()),
    ] {
        let err = lab
            .actions
            .run(&lab.uc, &caller(&carol), revoke_one(owner, &id))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, DomainError::NotFound(m) if m == &format!("Session '{id}' not found"))
        );
    }
    assert_eq!(lab.accounts.sessions(bob.id()), vec![bobs]);
    assert_eq!(lab.accounts.sessions(carol.id()), vec![expired]);
}
