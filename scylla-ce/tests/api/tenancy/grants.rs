//! The tests of `AccessService`: the roles that it gives and takes back, and the projects that
//! a role shows in the list.

use scylla_testkit::{text, Role};
use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::assert_refused;
use crate::identity::agents::agent_in;
use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// A viewer gets the project after `Grant`, and no more after `Revoke`.
#[sqlx::test(migrations = false)]
async fn a_role_gives_access_until_revoke(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let bob = app.user().await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let (id, to) = (text(&project.id), bob.to_string());
    let viewer = Role::on("project", id, "viewer");

    // when
    let before = app.get_project(bob, id).await;
    app.grant_role(owner, viewer, &to).await.unwrap();
    let granted = app.get_project(bob, id).await;
    app.revoke_role(owner, viewer, &to).await.unwrap();
    let revoked = app.get_project(bob, id).await;

    // then
    assert_code(before, Code::PermissionDenied);
    assert!(granted.is_ok(), "{granted:?}");
    assert_code(revoked, Code::PermissionDenied);
}

/// A viewer cannot give a role and cannot delete the project. An admin can do both.
#[sqlx::test(migrations = false)]
async fn each_role_gives_its_permissions(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let (viewer, admin) = (app.user().await, app.user().await);
    let other = app.user().await.to_string();
    let project = app.create_project(owner, "alpha").await.unwrap();
    let id = text(&project.id);
    let role = |name| Role::on("project", id, name);
    app.grant_role(owner, role("viewer"), &viewer.to_string())
        .await
        .unwrap();
    app.grant_role(owner, role("admin"), &admin.to_string())
        .await
        .unwrap();

    // when
    let viewer_grants = app.grant_role(viewer, role("viewer"), &other).await;
    let viewer_deletes = app.delete_project(viewer, id).await;
    let admin_grants = app.grant_role(admin, role("viewer"), &other).await;
    let admin_deletes = app.delete_project(admin, id).await;

    // then
    assert_code(viewer_grants, Code::PermissionDenied);
    assert_code(viewer_deletes, Code::PermissionDenied);
    assert!(admin_grants.is_ok(), "{admin_grants:?}");
    assert!(admin_deletes.is_ok(), "{admin_deletes:?}");
}

/// A stranger gives no role on a project or in a workspace, and an unknown project gives the
/// same answer. A stranger takes back no role.
#[sqlx::test(migrations = false)]
async fn grant_and_revoke_need_the_permission(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let stranger = app.user().await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let id = text(&project.id);
    let (workspace, owner_id, me) = (
        text(&project.workspace_id),
        owner.to_string(),
        stranger.to_string(),
    );

    // when
    let on_project = app
        .grant_role(stranger, Role::on("project", id, "admin"), &me)
        .await;
    let on_unknown = app
        .grant_role(stranger, Role::on("project", UNKNOWN_ID, "admin"), &me)
        .await;
    let in_workspace = app
        .grant_role(stranger, Role::on("workspace", workspace, "member"), &me)
        .await;
    let revoked = app
        .revoke_role(stranger, Role::on("project", id, "admin"), &owner_id)
        .await;

    // then
    assert_code(on_project, Code::PermissionDenied);
    assert_code(on_unknown, Code::PermissionDenied);
    assert_code(in_workspace, Code::PermissionDenied);
    assert_code(revoked, Code::PermissionDenied);
}

/// A bad kind, object id, role or actor id is a request error, and an unknown actor is not
/// found. A permission is not a role, and an actor has no roles.
#[sqlx::test(migrations = false)]
async fn grant_checks_the_request(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let bob = app.user().await.to_string();
    let project = app.create_project(owner, "alpha").await.unwrap();
    let id = text(&project.id);
    let viewer = Role::on("project", id, "viewer");

    // when
    let kind = app
        .grant_role(owner, Role::on("actor", &bob, "delegate"), &bob)
        .await;
    let bad_id = app
        .grant_role(owner, Role::on("project", "nope", "viewer"), &bob)
        .await;
    let role = app
        .grant_role(owner, Role::on("project", id, "view"), &bob)
        .await;
    let actor = app.grant_role(owner, viewer, "nope").await;
    let unknown = app.grant_role(owner, viewer, UNKNOWN_ID).await;

    // then
    assert_invalid_field(&kind.unwrap_err(), "object.kind");
    assert_invalid_field(&bad_id.unwrap_err(), "object.id");
    assert_invalid_field(&role.unwrap_err(), "role");
    assert_invalid_field(&actor.unwrap_err(), "actor_id");
    assert_code(unknown, Code::NotFound);
}

/// An admin of a workspace gives roles in it and on its projects. A member gives none, and the
/// admin of one project gives roles on that project only.
#[sqlx::test(migrations = false)]
async fn only_the_admins_give_roles(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let (admin, member, project_admin) = (app.user().await, app.user().await, app.user().await);
    let other = app.user().await.to_string();
    let workspace = app.workspace_of(owner).await.to_string();
    let first = app.create_project(owner, "first").await.unwrap();
    let second = app.create_project(owner, "second").await.unwrap();
    let in_workspace = |name| Role::on("workspace", &workspace, name);
    app.grant_role(owner, in_workspace("admin"), &admin.to_string())
        .await
        .unwrap();
    app.grant_role(owner, in_workspace("member"), &member.to_string())
        .await
        .unwrap();
    let first_admin = Role::on("project", text(&first.id), "admin");
    app.grant_role(owner, first_admin, &project_admin.to_string())
        .await
        .unwrap();

    // when
    let by_admin = app.grant_role(admin, in_workspace("viewer"), &other).await;
    let by_member = app.grant_role(member, in_workspace("viewer"), &other).await;
    let on_first = Role::on("project", text(&first.id), "viewer");
    let on_second = Role::on("project", text(&second.id), "viewer");
    let first_by_project_admin = app.grant_role(project_admin, on_first, &other).await;
    let second_by_project_admin = app.grant_role(project_admin, on_second, &other).await;

    // then
    assert!(by_admin.is_ok(), "{by_admin:?}");
    assert_code(by_member, Code::PermissionDenied);
    assert!(first_by_project_admin.is_ok(), "{first_by_project_admin:?}");
    assert_code(second_by_project_admin, Code::PermissionDenied);
}

/// The list shows the projects of the workspaces that the caller owns or where it has a role,
/// and the projects shared with it. `workspace_id` filters the list.
#[sqlx::test(migrations = false)]
async fn list_projects_shows_owned_member_and_shared_projects(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (alice, bob, carol) = (app.user().await, app.user().await, app.user().await);
    let to = alice.to_string();
    app.create_project(alice, "alice").await.unwrap();
    app.create_project(bob, "bob private").await.unwrap();
    let shared = app.create_project(bob, "bob shared").await.unwrap();
    app.create_project(carol, "carol").await.unwrap();
    let carol_workspace = app.workspace_of(carol).await.to_string();
    let viewer = Role::on("project", text(&shared.id), "viewer");
    app.grant_role(bob, viewer, &to).await.unwrap();
    let member = Role::on("workspace", &carol_workspace, "member");
    app.grant_role(carol, member, &to).await.unwrap();

    // when
    let all = app.visible_projects(alice, None).await.unwrap();
    let bob_workspace = app.workspace_of(bob).await;
    let of_bob = app
        .visible_projects(alice, Some(bob_workspace))
        .await
        .unwrap();

    // then
    assert_eq!(all, ["alice", "bob shared", "carol"]);
    assert_eq!(of_bob, ["bob shared"]);
}

/// An agent gets a role in its own workspace, on the workspace or on a project in it. An agent
/// gets no role in another workspace: `ACTOR_OUTSIDE_WORKSPACE`. A user gets a role in any
/// workspace.
#[sqlx::test(migrations = false)]
async fn grant_keeps_an_agent_in_its_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, other) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    let (agent, _) = agent_in(&app, owner, workspace).await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let foreign = app.create_project(other, "beta").await.unwrap();
    let (home, abroad) = (workspace.to_string(), text(&foreign.workspace_id));
    let (agent, user) = (agent.to_string(), owner.to_string());

    // when
    let in_home = app
        .grant_role(owner, Role::on("workspace", &home, "member"), &agent)
        .await;
    let on_project = app
        .grant_role(
            owner,
            Role::on("project", text(&project.id), "viewer"),
            &agent,
        )
        .await;
    let in_other = app
        .grant_role(other, Role::on("workspace", abroad, "viewer"), &agent)
        .await;
    let on_foreign = app
        .grant_role(
            other,
            Role::on("project", text(&foreign.id), "viewer"),
            &agent,
        )
        .await;
    let user_in_other = app
        .grant_role(other, Role::on("workspace", abroad, "viewer"), &user)
        .await;

    // then
    assert!(
        in_home.is_ok() && on_project.is_ok(),
        "{in_home:?} {on_project:?}"
    );
    assert_refused(in_other, "ACTOR_OUTSIDE_WORKSPACE");
    assert_refused(on_foreign, "ACTOR_OUTSIDE_WORKSPACE");
    assert!(user_in_other.is_ok(), "{user_in_other:?}");
}
