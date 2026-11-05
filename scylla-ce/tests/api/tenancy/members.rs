//! The tests of `ListMembers`, `GetMemberAccess`, `ListHeldPermissions` and `ListRoleHolders`.

use scylla_proto::access::v1::Member;
use scylla_testkit::{text, Role};
use sqlx::PgPool;
use tonic::Code;

use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// Returns the keys of the roles of an actor in a list of members, as `kind:key`.
fn roles_of(members: &[Member], actor: &str) -> Vec<String> {
    let member = members
        .iter()
        .find(|member| member.actor_id.as_ref().is_some_and(|id| id.value == actor));
    let mut roles: Vec<String> = member
        .map(|member| {
            member
                .roles
                .iter()
                .map(|role| format!("{}:{}", role.scope_kind, role.role_key))
                .collect()
        })
        .unwrap_or_default();
    roles.sort();
    roles
}

/// A workspace shows its owners, its roles, the roles on its projects, and the instance admins. A
/// grant tells its time and its author.
#[sqlx::test(migrations = false)]
async fn list_members_shows_each_role_of_a_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let (owner, bob) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    let project = app.create_project(owner, "alpha").await.unwrap();
    let to = bob.to_string();
    app.grant_role(owner, Role::on("workspace", &workspace, "member"), &to)
        .await
        .unwrap();
    app.grant_role(owner, Role::on("project", text(&project.id), "admin"), &to)
        .await
        .unwrap();

    // when
    let members = app
        .members_of(owner, "workspace", &workspace)
        .await
        .unwrap();

    // then
    assert_eq!(roles_of(&members, &owner.to_string()), ["workspace:owner"]);
    assert_eq!(
        roles_of(&members, &to),
        ["project:project-admin", "workspace:workspace-member"]
    );
    assert_eq!(
        roles_of(&members, &admin.to_string()),
        ["platform:instance-admin"]
    );
    let bob = members
        .iter()
        .find(|member| member.actor_id.as_ref().is_some_and(|id| id.value == to))
        .unwrap();
    let on_project = bob
        .roles
        .iter()
        .find(|role| role.scope_kind == "project")
        .unwrap();
    assert_eq!(on_project.scope_name, "alpha");
    assert!(on_project.granted_at.is_some());
    assert_eq!(
        on_project.granted_by.as_ref().unwrap().value,
        owner.to_string()
    );
}

/// A project shows its own roles, and those of the workspace. A person with no role on the object
/// gets a refusal.
#[sqlx::test(migrations = false)]
async fn list_members_of_a_project_shows_the_roles_that_reach_it(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, bob, stranger) = (app.user().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    let first = app.create_project(owner, "first").await.unwrap();
    let second = app.create_project(owner, "second").await.unwrap();
    let to = bob.to_string();
    app.grant_role(owner, Role::on("workspace", &workspace, "viewer"), &to)
        .await
        .unwrap();
    app.grant_role(owner, Role::on("project", text(&first.id), "editor"), &to)
        .await
        .unwrap();
    app.grant_role(owner, Role::on("project", text(&second.id), "admin"), &to)
        .await
        .unwrap();

    // when
    let members = app
        .members_of(owner, "project", text(&first.id))
        .await
        .unwrap();
    let refused = app.members_of(stranger, "project", text(&first.id)).await;
    let unknown = app.members_of(owner, "project", UNKNOWN_ID).await;

    // then
    assert_eq!(
        roles_of(&members, &to),
        ["project:project-editor", "workspace:workspace-viewer"]
    );
    assert_eq!(roles_of(&members, &owner.to_string()), ["workspace:owner"]);
    assert_code(refused, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
}

/// The effective access names the roles that give each permission, on the project and above it.
#[sqlx::test(migrations = false)]
async fn member_access_names_the_source_of_each_permission(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, bob) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    let project = app.create_project(owner, "alpha").await.unwrap();
    let other = app.create_project(owner, "other").await.unwrap();
    let to = bob.to_string();
    app.grant_role(owner, Role::on("workspace", &workspace, "viewer"), &to)
        .await
        .unwrap();
    app.grant_role(owner, Role::on("project", text(&project.id), "editor"), &to)
        .await
        .unwrap();
    app.grant_role(owner, Role::on("project", text(&other.id), "admin"), &to)
        .await
        .unwrap();

    // when
    let on_project = app
        .member_access(owner, &to, "project", text(&project.id))
        .await
        .unwrap();
    let on_workspace = app
        .member_access(owner, &to, "workspace", &workspace)
        .await
        .unwrap();
    let by_stranger = app
        .member_access(app.user().await, &to, "workspace", &workspace)
        .await;

    // then
    let sources = |answer: &scylla_proto::access::v1::GetMemberAccessResponse, id: &str| {
        let mut found: Vec<String> = answer
            .permissions
            .iter()
            .find(|permission| permission.permission == id)
            .map(|permission| {
                permission
                    .sources
                    .iter()
                    .map(|source| source.role_key.clone())
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        found
    };
    assert_eq!(
        sources(&on_project, "project.view"),
        ["project-editor", "workspace-viewer"]
    );
    assert_eq!(sources(&on_project, "pipelines.edit"), ["project-editor"]);
    assert!(sources(&on_project, "project.delete").is_empty());
    assert_eq!(sources(&on_workspace, "project.view"), ["workspace-viewer"]);
    assert!(sources(&on_workspace, "pipelines.edit").is_empty());
    let roles = on_workspace.member.unwrap().roles;
    assert_eq!(roles.len(), 3);
    assert_code(by_stranger, Code::PermissionDenied);
}

/// An owner holds each permission of a workspace. An editor holds part of a project, and nothing on
/// another object.
#[sqlx::test(migrations = false)]
async fn list_held_permissions_tells_what_the_caller_can_give(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, bob) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    let project = app.create_project(owner, "alpha").await.unwrap();
    let to = bob.to_string();
    app.grant_role(owner, Role::on("workspace", &workspace, "viewer"), &to)
        .await
        .unwrap();
    app.grant_role(owner, Role::on("project", text(&project.id), "editor"), &to)
        .await
        .unwrap();

    // when
    let by_owner = app
        .held_permissions(owner, "workspace", &workspace)
        .await
        .unwrap();
    let on_project = app
        .held_permissions(bob, "project", text(&project.id))
        .await
        .unwrap();
    let unknown = app
        .held_permissions(bob, "project", UNKNOWN_ID)
        .await
        .unwrap();
    let bad = app.held_permissions(bob, "pipeline", UNKNOWN_ID).await;

    // then
    assert!(by_owner.iter().any(|id| id == "workspace.delete"));
    assert!(on_project.iter().any(|id| id == "pipelines.edit"));
    assert!(!on_project.iter().any(|id| id == "project.delete"));
    assert!(unknown.is_empty());
    assert_invalid_field(&bad.unwrap_err(), "object.kind");
}

/// A list for a workspace counts only its grants, and the holders tell their scope.
#[sqlx::test(migrations = false)]
async fn holders_count_for_one_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, other, bob) = (app.user().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    let abroad = app.workspace_of(other).await.to_string();
    let to = bob.to_string();
    app.grant_role(owner, Role::on("workspace", &workspace, "member"), &to)
        .await
        .unwrap();
    app.grant_role(other, Role::on("workspace", &abroad, "member"), &to)
        .await
        .unwrap();

    // when
    let roles = app.list_roles(owner, Some(&workspace)).await.unwrap();
    let holders = app
        .role_holders(owner, "workspace-member", Some(&workspace))
        .await
        .unwrap();
    let unknown = app.role_holders(owner, UNKNOWN_ID, Some(&workspace)).await;
    let instance = app.role_holders(owner, "workspace-member", None).await;

    // then
    let member = roles
        .iter()
        .find(|role| role.key == "workspace-member")
        .unwrap();
    assert_eq!(member.holder_count, 1);
    assert_eq!(holders.total, 1);
    assert_eq!(holders.holders[0].actor_id.as_ref().unwrap().value, to);
    assert_eq!(holders.holders[0].scope_kind, "workspace");
    assert_code(unknown, Code::PermissionDenied);
    assert_code(instance, Code::PermissionDenied);
}
