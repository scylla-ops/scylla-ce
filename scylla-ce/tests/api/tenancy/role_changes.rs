//! The tests of the changes of a role: a built-in role stays, a role in use stays, and a change is
//! live at once.

use scylla_domain::identity::actors::ActorId;
use scylla_testkit::{RoleInput, TestApp};
use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::assert_refused;
use crate::{app, assert_code, UNKNOWN_ID};

/// A built-in role has no id, so it does not change, and an unknown role gets a refusal.
#[sqlx::test(migrations = false)]
async fn update_and_delete_refuse_a_built_in_or_unknown_role(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;

    // when
    let update = app.update_role(admin, "instance-admin", "Other", &[]).await;
    let delete = app.delete_role(admin, "instance-admin").await;
    let unknown_update = app.update_role(admin, UNKNOWN_ID, "Other", &[]).await;
    let unknown_delete = app.delete_role(admin, UNKNOWN_ID).await;

    // then
    assert_code(update, Code::InvalidArgument);
    assert_code(delete, Code::InvalidArgument);
    assert_code(unknown_update, Code::PermissionDenied);
    assert_code(unknown_delete, Code::PermissionDenied);
}

/// A role that an actor holds stays, and a role with no holder goes.
#[sqlx::test(migrations = false)]
async fn delete_role_refuses_a_role_that_an_actor_holds(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, user) = (app.admin().await, app.user().await);
    let input = RoleInput::on_platform("Auditor", &["users.view"]);
    let role = app.create_role(admin, input).await.unwrap();
    app.grant_on_platform(admin, &role.grant_name, user)
        .await
        .unwrap();

    // when
    let refused = app.delete_role(admin, &role.key).await;
    app.revoke_on_platform(admin, &role.grant_name, user)
        .await
        .unwrap();
    let deleted = app.delete_role(admin, &role.key).await;

    // then
    assert_refused(refused, "ROLE_IN_USE");
    assert!(deleted.is_ok());
    assert_code(
        app.get_role(admin, &role.key, None).await,
        Code::PermissionDenied,
    );
}

/// A workspace owns its own roles: they are in its list and not in the list of the instance.
#[sqlx::test(migrations = false)]
async fn a_workspace_makes_its_own_roles(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, owner, other) = (app.admin().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    let input = RoleInput {
        name: "Release",
        level: "workspace",
        holder: "people",
        permissions: &["pipelines.run"],
        workspace: Some(&workspace),
    };

    // when
    let role = app.create_role(owner, input).await.unwrap();
    let refused = app.create_role(other, input).await;
    let platform = RoleInput {
        level: "platform",
        ..input
    };
    let on_platform = app.create_role(owner, platform).await;
    let for_workspace = app.list_roles(owner, Some(&workspace)).await.unwrap();
    let for_instance = app.list_roles(admin, None).await.unwrap();

    // then
    assert_eq!(
        role.permissions,
        ["workspace.view", "pipelines.view", "pipelines.run"]
    );
    assert_code(refused, Code::PermissionDenied);
    assert_code(on_platform, Code::InvalidArgument);
    assert!(for_workspace.iter().any(|other| other.key == role.key));
    assert!(!for_workspace.iter().any(|other| other.level == "platform"));
    assert!(!for_instance.iter().any(|other| other.key == role.key));
}

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

/// A change of a role is live at once for its holders.
#[sqlx::test(migrations = false)]
async fn update_role_is_live_at_once(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let input = RoleInput::on_platform("Auditor", &["users.view"]);
    let role = app.create_role(admin, input).await.unwrap();
    let user = app.user().await;
    app.grant_on_platform(admin, &role.grant_name, user)
        .await
        .unwrap();
    let page = scylla_proto::common::v1::PageRequest::default();
    assert!(app.list_users(user, "", page.clone()).await.is_ok());

    // when
    app.update_role(admin, &role.key, "Auditor", &[])
        .await
        .unwrap();

    // then
    assert_code(app.list_users(user, "", page).await, Code::PermissionDenied);
}

/// A role of the instance gives its routes, and only them.
#[sqlx::test(migrations = false)]
async fn a_role_of_the_instance_gives_its_routes(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let user = user_with_role(&app, admin, "Settings", &["instance_settings.manage"]).await;

    // when
    let settings = app.get_instance_settings(user).await;
    let page = scylla_proto::common::v1::PageRequest::default();
    let users = app.list_users(user, "", page).await;

    // then
    assert!(settings.is_ok());
    assert_code(users, Code::PermissionDenied);
}
