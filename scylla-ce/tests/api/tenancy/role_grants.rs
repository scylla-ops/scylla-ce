//! The tests of a custom role on a workspace and on a project: `Grant`, `Revoke`, what the holder
//! can do, and who can give the role.

use scylla_testkit::{text, Role, RoleInput};
use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::assert_refused;
use crate::{app, assert_code};

/// A role of a workspace gives its permissions on the workspace and on its projects, until
/// `Revoke`.
#[sqlx::test(migrations = false)]
async fn a_custom_workspace_role_gives_its_permissions(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, bob) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    let project = app.create_project(owner, "alpha").await.unwrap();
    let input = RoleInput {
        name: "Reader",
        level: "workspace",
        holder: "people",
        permissions: &["project.view"],
        workspace: Some(&workspace),
    };
    let role = app.create_role(owner, input).await.unwrap();
    let grant = Role::on("workspace", &workspace, &role.grant_name);

    // when
    let before = app.get_project(bob, text(&project.id)).await;
    app.grant_role(owner, grant, &bob.to_string())
        .await
        .unwrap();
    let granted = app.get_project(bob, text(&project.id)).await;
    let entries = app
        .access_of(owner, "workspace", &workspace, 2)
        .await
        .unwrap();
    app.revoke_role(owner, grant, &bob.to_string())
        .await
        .unwrap();
    let revoked = app.get_project(bob, text(&project.id)).await;

    // then
    assert_code(before, Code::PermissionDenied);
    assert!(granted.is_ok(), "{granted:?}");
    assert!(entries.contains(&(bob.to_string(), role.grant_name.clone())));
    assert_code(revoked, Code::PermissionDenied);
}

/// A role of another workspace, a role of another level, and a text that is not a role are request
/// errors.
#[sqlx::test(migrations = false)]
async fn grant_refuses_a_role_that_the_object_cannot_use(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, other, bob) = (app.user().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    let abroad = app.workspace_of(other).await.to_string();
    let project = app.create_project(owner, "alpha").await.unwrap();
    let foreign = RoleInput {
        name: "Foreign",
        level: "workspace",
        holder: "people",
        permissions: &["project.view"],
        workspace: Some(&abroad),
    };
    let foreign = app.create_role(other, foreign).await.unwrap();
    let local = RoleInput {
        name: "Local",
        level: "workspace",
        holder: "people",
        permissions: &["project.view"],
        workspace: Some(&workspace),
    };
    let local = app.create_role(owner, local).await.unwrap();
    let to = bob.to_string();

    // when
    let of_other = app
        .grant_role(
            owner,
            Role::on("workspace", &workspace, &foreign.grant_name),
            &to,
        )
        .await;
    let of_other_level = app
        .grant_role(
            owner,
            Role::on("project", text(&project.id), &local.grant_name),
            &to,
        )
        .await;
    let not_a_role = app
        .grant_role(owner, Role::on("workspace", &workspace, "nope"), &to)
        .await;

    // then
    assert_code(of_other, Code::InvalidArgument);
    assert_code(of_other_level, Code::InvalidArgument);
    assert_code(not_a_role, Code::InvalidArgument);
}

/// A role for apps does not go to a user, and a custom role for people does not go to an app.
#[sqlx::test(migrations = false)]
async fn grant_keeps_each_role_with_its_holders(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, bob) = (app.user().await, app.user().await);
    let id = app.workspace_of(owner).await;
    let workspace = id.to_string();
    let agent = app.app_in(owner, id, true).await.0.actor();
    let input = RoleInput {
        name: "People",
        level: "workspace",
        holder: "people",
        permissions: &["project.view"],
        workspace: Some(&workspace),
    };
    let people = app.create_role(owner, input).await.unwrap();

    // when
    let runner = app
        .grant_role(
            owner,
            Role::on("workspace", &workspace, "runner"),
            &bob.to_string(),
        )
        .await;
    let to_app = app
        .grant_role(
            owner,
            Role::on("workspace", &workspace, &people.grant_name),
            &agent.to_string(),
        )
        .await;

    // then
    assert_code(runner, Code::InvalidArgument);
    assert_code(to_app, Code::InvalidArgument);
}

/// A holder of `members.manage` gives only the roles that it holds: a built-in admin gives
/// `ESCALATION`, and a role in its own permissions passes.
#[sqlx::test(migrations = false)]
async fn grant_refuses_a_role_above_the_caller(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, manager, bob) = (app.user().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    let input = RoleInput {
        name: "Manager",
        level: "workspace",
        holder: "people",
        permissions: &["members.manage", "workspace.view", "project.view"],
        workspace: Some(&workspace),
    };
    let role = app.create_role(owner, input).await.unwrap();
    let small = RoleInput {
        name: "Small",
        level: "workspace",
        holder: "people",
        permissions: &["project.view"],
        workspace: Some(&workspace),
    };
    let small = app.create_role(owner, small).await.unwrap();
    app.grant_role(
        owner,
        Role::on("workspace", &workspace, &role.grant_name),
        &manager.to_string(),
    )
    .await
    .unwrap();
    let to = bob.to_string();

    // when
    let admin = app
        .grant_role(manager, Role::on("workspace", &workspace, "admin"), &to)
        .await;
    let allowed = app
        .grant_role(
            manager,
            Role::on("workspace", &workspace, &small.grant_name),
            &to,
        )
        .await;

    // then
    assert_refused(admin, "ESCALATION");
    assert!(allowed.is_ok(), "{allowed:?}");
}

/// The delete of a project takes its grants that are rows, so the id does not count as a holder.
#[sqlx::test(migrations = false)]
async fn delete_of_a_project_takes_its_grants(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, bob) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    let project = app.create_project(owner, "alpha").await.unwrap();
    let input = RoleInput {
        name: "On project",
        level: "project",
        holder: "people",
        permissions: &["project.view"],
        workspace: Some(&workspace),
    };
    let role = app.create_role(owner, input).await.unwrap();
    app.grant_role(
        owner,
        Role::on("workspace", &workspace, "viewer"),
        &bob.to_string(),
    )
    .await
    .unwrap();
    app.grant_role(
        owner,
        Role::on("project", text(&project.id), &role.grant_name),
        &bob.to_string(),
    )
    .await
    .unwrap();

    // when
    let before = app
        .role_holders(owner, &role.key, Some(&workspace))
        .await
        .unwrap();
    app.delete_project(owner, text(&project.id)).await.unwrap();
    let after = app
        .role_holders(owner, &role.key, Some(&workspace))
        .await
        .unwrap();
    let delete = app.delete_role(owner, &role.key).await;

    // then
    assert_eq!(before.total, 1);
    assert_eq!(after.total, 0);
    assert!(delete.is_ok(), "{delete:?}");
}
