//! The tests of the roles of the whole instance: `Grant` and `Revoke` on the platform, what these
//! roles reach, and the console of the users.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::common::v1::PageRequest;
use scylla_testkit::{text, RoleInput, TestApp};
use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::assert_refused;
use crate::{app, assert_code};

/// Makes a role of the instance, and gives it to a new user.
async fn user_with_role(
    app: &TestApp,
    admin: ActorId,
    name: &str,
    permissions: &[&str],
) -> ActorId {
    let user = app.user().await;
    let role = app
        .create_role(admin, RoleInput::on_platform(name, permissions))
        .await
        .unwrap();
    app.grant_on_platform(admin, &role.grant_name, user)
        .await
        .unwrap();
    user
}

/// `Grant` on the platform needs a right, a known role of the platform level and a user.
#[sqlx::test(migrations = false)]
async fn grant_on_platform_refuses_a_bad_request(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, user) = (app.admin().await, app.user().await);
    let workspace = app.workspace_of(user).await;
    let machine = app.create_app(user, workspace, false).await.unwrap();
    let machine = machine.app.expect("app in response");
    let machine = ActorId::parse(text(&machine.id)).unwrap();

    // when
    let role = RoleInput::on_platform("Auditor", &["users.view"]);
    let role = app.create_role(admin, role).await.unwrap();
    let no_right = app.grant_on_platform(user, &role.grant_name, user).await;
    let not_a_role = app.grant_on_platform(admin, "pilot", user).await;
    let of_a_workspace = app.grant_on_platform(admin, "workspace-member", user).await;
    let an_app = app
        .grant_on_platform(admin, &role.grant_name, machine)
        .await;

    // then
    assert_code(no_right, Code::PermissionDenied);
    assert_code(not_a_role, Code::InvalidArgument);
    assert_code(of_a_workspace, Code::InvalidArgument);
    assert_refused(an_app, "ACTOR_NOT_A_USER");
}

/// `Grant` of the role of the instance admins makes an admin, and the last admin stays.
#[sqlx::test(migrations = false)]
async fn the_role_of_the_instance_admins_keeps_an_admin(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, user) = (app.admin().await, app.user().await);

    // when
    app.grant_on_platform(admin, "instance-admin", user)
        .await
        .unwrap();
    let page = PageRequest::default();
    let reached = app.list_users(user, "", page.clone()).await;
    app.revoke_on_platform(admin, "instance-admin", user)
        .await
        .unwrap();
    let last = app.revoke_on_platform(admin, "instance-admin", admin).await;

    // then
    assert!(reached.is_ok());
    assert_refused(last, "LAST_ADMIN");
    assert_code(app.list_users(user, "", page).await, Code::PermissionDenied);
}

/// A holder of the right to give roles cannot give a role with more than it holds, and cannot
/// give or take the role of the instance admins.
#[sqlx::test(migrations = false)]
async fn grant_on_platform_refuses_an_escalation(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let keeper = user_with_role(&app, admin, "Keeper", &["instance_access.manage"]).await;
    let wide = RoleInput::on_platform("Wide", &["users.delete"]);
    let wide = app.create_role(admin, wide).await.unwrap();
    let narrow = RoleInput::on_platform("Narrow", &["instance_access.manage"]);
    let narrow = app.create_role(admin, narrow).await.unwrap();
    let user = app.user().await;

    // when
    let refused = app.grant_on_platform(keeper, &wide.grant_name, user).await;
    let admin_role = app.grant_on_platform(keeper, "instance-admin", user).await;
    let take_back = app
        .revoke_on_platform(keeper, "instance-admin", admin)
        .await;
    let accepted = app
        .grant_on_platform(keeper, &narrow.grant_name, user)
        .await;

    // then
    assert_refused(refused, "ESCALATION");
    assert_refused(admin_role, "ESCALATION");
    assert_refused(take_back, "ESCALATION");
    assert!(accepted.is_ok());
}

/// An instance admin has full control of each workspace and of each project.
#[sqlx::test(migrations = false)]
async fn an_instance_admin_reaches_each_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, owner) = (app.admin().await, app.user().await);
    let team = app.create_workspace(owner, "team").await.unwrap();
    let project = app.create_project(owner, "alpha").await.unwrap();

    // when
    let read = app.get_workspace(admin, text(&team.id)).await;
    let workspaces = app.visible_workspaces(admin).await.unwrap();
    let projects = app.visible_projects(admin, None).await.unwrap();
    let deleted = app.delete_project(admin, text(&project.id)).await;

    // then
    assert!(read.is_ok());
    assert!(workspaces.contains(&"team".to_owned()));
    assert!(projects.contains(&"alpha".to_owned()));
    assert!(deleted.is_ok());
}

/// A role of the instance reaches the workspaces and the projects with its permissions, and it
/// changes nothing that it does not hold.
#[sqlx::test(migrations = false)]
async fn a_role_of_the_instance_reaches_each_project(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, owner) = (app.admin().await, app.user().await);
    let auditor =
        user_with_role(&app, admin, "Auditor", &["workspaces.view", "project.view"]).await;
    let plain = app.user().await;
    app.create_workspace(owner, "team").await.unwrap();
    let project = app.create_project(owner, "alpha").await.unwrap();

    // when
    let workspaces = app.visible_workspaces(auditor).await.unwrap();
    let projects = app.visible_projects(auditor, None).await.unwrap();
    let read = app.get_project(auditor, text(&project.id)).await;
    let write = app.delete_project(auditor, text(&project.id)).await;
    let other = app.visible_workspaces(plain).await.unwrap();

    // then
    assert!(workspaces.contains(&"team".to_owned()));
    assert!(projects.contains(&"alpha".to_owned()));
    assert!(read.is_ok());
    assert_code(write, Code::PermissionDenied);
    assert!(!other.contains(&"team".to_owned()));
}

/// A role of the instance with a permission in the projects also gets the view of all the
/// workspaces, so its holder finds each workspace where it acts.
#[sqlx::test(migrations = false)]
async fn a_role_of_the_instance_that_acts_in_projects_finds_each_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, owner, user) = (app.admin().await, app.user().await, app.user().await);
    let input = RoleInput::on_platform("Pipelines", &["pipelines.view"]);
    let role = app.create_role(admin, input).await.unwrap();
    app.grant_on_platform(admin, &role.grant_name, user)
        .await
        .unwrap();
    app.create_workspace(owner, "team").await.unwrap();

    // when
    let workspaces = app.visible_workspaces(user).await.unwrap();

    // then
    assert!(role.permissions.contains(&"workspaces.view".to_owned()));
    assert!(workspaces.contains(&"team".to_owned()));
}

/// `CheckPermissions` on the platform tells what a role gives, as a route check does.
#[sqlx::test(migrations = false)]
async fn check_permissions_sees_a_role_of_the_instance(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let user = user_with_role(&app, admin, "Auditor", &["users.view"]).await;
    let platform = scylla_domain::platform::Platform::ID.to_string();

    // when
    let answers = app
        .check_permissions(user, "platform", &platform, &["list_users", "delete_user"])
        .await
        .unwrap();

    // then
    assert_eq!(answers, [true, false]);
}

/// `ListUsers` tells the instance roles, the workspaces and the last sign-in of each user, and
/// counts them.
#[sqlx::test(migrations = false)]
async fn list_users_tells_the_place_of_each_user(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let holder = user_with_role(&app, admin, "Auditor", &["users.view"]).await;
    let plain = app.user().await;
    let workspace = app.workspace_of(plain).await.to_string();
    let role = scylla_testkit::Role::on("workspace", &workspace, "viewer");
    app.grant_role(plain, role, &holder.to_string())
        .await
        .unwrap();
    let page = PageRequest::default();

    // when
    let all = app.list_users(admin, "", page.clone()).await.unwrap();
    let kept = app.list_users_with_role(admin, page).await.unwrap();

    // then
    let entry = |id: ActorId| {
        all.users
            .iter()
            .find(|entry| text(&entry.user.as_ref().unwrap().id) == id.to_string())
            .unwrap()
    };
    let roles = &entry(holder).instance_roles;
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].name, "Auditor");
    assert!(!roles[0].builtin);
    assert_eq!(entry(admin).instance_roles[0].key, "instance-admin");
    assert!(entry(plain).instance_roles.is_empty());
    assert_eq!(entry(holder).workspaces.len(), 2);
    assert_eq!(all.total, 3);
    assert_eq!(all.total_with_instance_role, 2);
    assert_eq!(kept.users.len(), 2);
}

/// `DeleteUser` removes the grants of the user, so its role can go.
#[sqlx::test(migrations = false)]
async fn delete_user_removes_its_grants(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let role = RoleInput::on_platform("Auditor", &["users.view"]);
    let role = app.create_role(admin, role).await.unwrap();
    let user = app.user().await;
    app.grant_on_platform(admin, &role.grant_name, user)
        .await
        .unwrap();

    // when
    app.delete_user(admin, &user.to_string()).await.unwrap();
    let deleted = app.delete_role(admin, &role.key).await;

    // then
    assert!(deleted.is_ok());
}
