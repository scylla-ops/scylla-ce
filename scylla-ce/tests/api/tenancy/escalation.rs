//! The tests of the rule of the escalation: a caller gives only what it holds.

use scylla_testkit::{Role, RoleInput};
use sqlx::PgPool;

use crate::app;
use crate::identity::accounts::assert_refused;

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

/// A holder of `members.manage` invites only with a role that it holds: a built-in admin gives
/// `ESCALATION`.
#[sqlx::test(migrations = false)]
async fn an_invitation_refuses_a_role_above_the_caller(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, manager) = (app.user().await, app.user().await);
    let id = app.workspace_of(owner).await;
    let workspace = id.to_string();
    let input = RoleInput {
        name: "Manager",
        level: "workspace",
        holder: "people",
        permissions: &["members.manage", "workspace.view"],
        workspace: Some(&workspace),
    };
    let role = app.create_role(owner, input).await.unwrap();
    app.grant_role(
        owner,
        Role::on("workspace", &workspace, &role.grant_name),
        &manager.to_string(),
    )
    .await
    .unwrap();

    // when
    let refused = app
        .create_invitation(manager, id, "new@example.com", "admin")
        .await;
    let by_owner = app
        .create_invitation(owner, id, "new@example.com", "admin")
        .await;

    // then
    assert_refused(refused, "ESCALATION");
    assert!(by_owner.is_ok(), "{by_owner:?}");
}

/// No person holds the permissions of a machine, so a Workspace admin gives the role of an agent.
#[sqlx::test(migrations = false)]
async fn an_admin_gives_the_role_of_an_agent(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, admin) = (app.user().await, app.user().await);
    let id = app.workspace_of(owner).await;
    let workspace = id.to_string();
    let agent = app.app_in(owner, id, false).await.0.actor();
    app.grant_role(
        owner,
        Role::on("workspace", &workspace, "admin"),
        &admin.to_string(),
    )
    .await
    .unwrap();

    // when
    let given = app
        .grant_role(
            admin,
            Role::on("workspace", &workspace, "runner"),
            &agent.to_string(),
        )
        .await;

    // then
    assert!(given.is_ok(), "{given:?}");
}
