//! The tests of a person in a workspace: a role on a project needs a role in the workspace, and the
//! roles on the projects go with the last role in the workspace.

use scylla_testkit::{text, Role, RoleInput};
use sqlx::PgPool;

use crate::app;
use crate::identity::accounts::assert_refused;

/// A user needs a role in the workspace before a role on a project: `NO_WORKSPACE_ROLE`. An app
/// needs none.
#[sqlx::test(migrations = false)]
async fn a_project_role_needs_a_workspace_role(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, bob) = (app.user().await, app.user().await);
    let id = app.workspace_of(owner).await;
    let workspace = id.to_string();
    let agent = app.app_in(owner, id, true).await.0.actor();
    let project = app.create_project(owner, "alpha").await.unwrap();
    let viewer = Role::on("project", text(&project.id), "viewer");

    // when
    let alone = app.grant_role(owner, viewer, &bob.to_string()).await;
    let to_agent = app.grant_role(owner, viewer, &agent.to_string()).await;
    app.grant_role(
        owner,
        Role::on("workspace", &workspace, "viewer"),
        &bob.to_string(),
    )
    .await
    .unwrap();
    let after = app.grant_role(owner, viewer, &bob.to_string()).await;

    // then
    assert_refused(alone, "NO_WORKSPACE_ROLE");
    assert!(to_agent.is_ok(), "{to_agent:?}");
    assert!(after.is_ok(), "{after:?}");
}

/// When the last workspace role goes, the roles on the projects go too. One other role keeps them.
#[sqlx::test(migrations = false)]
async fn the_last_workspace_role_takes_the_project_roles(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, bob) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    let project = app.create_project(owner, "alpha").await.unwrap();
    let to = bob.to_string();
    let input = RoleInput {
        name: "On project",
        level: "project",
        holder: "people",
        permissions: &["project.view"],
        workspace: Some(&workspace),
    };
    let row = app.create_role(owner, input).await.unwrap();
    let member = Role::on("workspace", &workspace, "member");
    let viewer = Role::on("workspace", &workspace, "viewer");
    app.grant_role(owner, member, &to).await.unwrap();
    app.grant_role(owner, viewer, &to).await.unwrap();
    let id = text(&project.id);
    app.grant_role(owner, Role::on("project", id, "editor"), &to)
        .await
        .unwrap();
    app.grant_role(owner, Role::on("project", id, &row.grant_name), &to)
        .await
        .unwrap();

    // when
    app.revoke_role(owner, member, &to).await.unwrap();
    let kept = app.access_of(owner, "project", id, 10).await.unwrap();
    app.revoke_role(owner, viewer, &to).await.unwrap();
    let gone = app.access_of(owner, "project", id, 10).await.unwrap();

    // then
    assert_eq!(kept.len(), 2);
    assert!(gone.is_empty(), "{gone:?}");
}

/// An owner that leaves with no other role also loses the roles on the projects.
#[sqlx::test(migrations = false)]
async fn an_owner_that_leaves_loses_the_project_roles(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, second) = (app.user().await, app.user().await);
    let id = app.workspace_of(owner).await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    app.add_owner(owner, &id.to_string(), &second.to_string())
        .await
        .unwrap();
    let editor = Role::on("project", text(&project.id), "editor");
    app.grant_role(owner, editor, &second.to_string())
        .await
        .unwrap();

    // when
    app.remove_owner(owner, &id.to_string(), &second.to_string())
        .await
        .unwrap();
    let left = app
        .access_of(owner, "project", text(&project.id), 10)
        .await
        .unwrap();

    // then
    assert!(left.is_empty(), "{left:?}");
}
