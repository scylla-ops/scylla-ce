//! The tests of the role owner of a workspace: a workspace has one owner or more, only an owner or
//! an instance admin gives and takes back the role, and the last owner stays.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::grants::OWNER;
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_proto::access::v1 as pb;
use scylla_testkit::{text, Role, RoleInput, TestApp};
use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::assert_refused;
use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// An owner gives the role to a user: the new owner gets each right, and the holders of the role
/// are the two owners. A second grant changes nothing.
#[sqlx::test(migrations = false)]
async fn grant_owner_gives_each_right(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (alice, bob) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(alice).await;
    let project = app.create_project(alice, "alpha").await.unwrap();
    let (id, bob_id) = (workspace.to_string(), bob.to_string());
    let check = || Workspace::MANAGE_OWNERS.on(workspace);

    // when
    let before = app.can(bob, check()).await;
    app.grant_role(alice, Role::owner_of(&id), &bob_id)
        .await
        .unwrap();
    app.grant_role(alice, Role::owner_of(&id), &bob_id)
        .await
        .unwrap();
    let after = app.can(bob, check()).await;
    let deleted = app.delete_project(bob, text(&project.id)).await;
    let holders = app.role_holders(bob, OWNER, Some(&id)).await.unwrap();

    // then
    assert!(!before && after);
    assert!(deleted.is_ok());
    assert_eq!(holders.total, 2);
}

/// A workspace admin, a viewer, another user and a custom role with each permission cannot change
/// the owners: no role gives `manage_owners`. An unknown workspace gets the same refusal.
#[sqlx::test(migrations = false)]
async fn only_an_owner_changes_the_owners(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, admin, viewer) = (app.user().await, app.user().await, app.user().await);
    let (stranger, custom) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[
        Workspace::ADMIN.of(workspace, admin),
        Workspace::VIEWER.of(workspace, viewer),
    ])
    .await;
    let (id, owner_id) = (workspace.to_string(), owner.to_string());
    let every = every_permission(&app, owner, &id).await;
    app.grant_role(
        owner,
        Role::on("workspace", &id, &every),
        &custom.to_string(),
    )
    .await
    .unwrap();
    let role = Role::owner_of(&id);

    // when
    let added_by_admin = app.grant_role(admin, role, &admin.to_string()).await;
    let added_by_viewer = app.grant_role(viewer, role, &viewer.to_string()).await;
    let added_by_custom = app.grant_role(custom, role, &custom.to_string()).await;
    let removed_by_admin = app.revoke_role(admin, role, &owner_id).await;
    let removed_by_custom = app.revoke_role(custom, role, &owner_id).await;
    let removed_by_stranger = app.revoke_role(stranger, role, &owner_id).await;
    let unknown = app
        .grant_role(owner, Role::owner_of(UNKNOWN_ID), &admin.to_string())
        .await;

    // then
    assert_code(added_by_admin, Code::PermissionDenied);
    assert_code(added_by_viewer, Code::PermissionDenied);
    assert_code(added_by_custom, Code::PermissionDenied);
    assert_code(removed_by_admin, Code::PermissionDenied);
    assert_code(removed_by_custom, Code::PermissionDenied);
    assert_code(removed_by_stranger, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
    assert_eq!(app.owner_count(workspace).await, 1);
}

/// Makes a custom role of the workspace with each permission of its level, and returns its name in
/// `Grant`.
async fn every_permission(app: &TestApp, owner: ActorId, workspace: &str) -> String {
    let built_in = app.get_role(owner, OWNER, Some(workspace)).await.unwrap();
    let ids: Vec<&str> = built_in.permissions.iter().map(String::as_str).collect();
    let input = RoleInput {
        name: "Every permission",
        level: "workspace",
        holder: "people",
        permissions: &ids,
        workspace: Some(workspace),
    };
    app.create_role(owner, input).await.unwrap().grant_name
}

/// An instance admin gives and takes back the role, with no role in the workspace.
#[sqlx::test(migrations = false)]
async fn an_instance_admin_changes_the_owners(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, owner, bob) = (app.admin().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    let (id, bob_id) = (workspace.to_string(), bob.to_string());

    // when
    let given = app.grant_role(admin, Role::owner_of(&id), &bob_id).await;
    let two = app.owner_count(workspace).await;
    let taken = app.revoke_role(admin, Role::owner_of(&id), &bob_id).await;

    // then
    assert!(given.is_ok(), "{given:?}");
    assert_eq!(two, 2);
    assert!(taken.is_ok(), "{taken:?}");
    assert_eq!(app.owner_count(workspace).await, 1);
}

/// An owner takes back the role from another owner, who loses its rights at once. The last owner
/// stays, and a user that is not an owner changes nothing.
#[sqlx::test(migrations = false)]
async fn revoke_owner_keeps_the_last_owner(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (alice, bob) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(alice).await;
    let (id, alice_id, bob_id) = (workspace.to_string(), alice.to_string(), bob.to_string());
    let role = Role::owner_of(&id);
    app.grant_role(alice, role, &bob_id).await.unwrap();

    // when
    app.revoke_role(bob, role, &alice_id).await.unwrap();
    let alice_reads = app.can(alice, Workspace::GET_WORKSPACE.on(workspace)).await;
    let last = app.revoke_role(bob, role, &bob_id).await;
    let not_an_owner = app.revoke_role(bob, role, &alice_id).await;

    // then
    assert!(!alice_reads);
    assert_refused(last, "LAST_OWNER");
    assert!(not_an_owner.is_ok());
    assert_eq!(app.owner_count(workspace).await, 1);
    assert!(app.can(bob, Workspace::MANAGE_OWNERS.on(workspace)).await);
}

/// Only a user holds the role: an unknown actor gives `NOT_FOUND`, an agent `INVALID_ARGUMENT`, and
/// a value that is not an id `INVALID_ARGUMENT`. The role is not a role of a project.
#[sqlx::test(migrations = false)]
async fn grant_owner_needs_a_user_on_a_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let agent = app.app_in(owner, workspace, true).await.0.actor();
    let project = app.create_project(owner, "alpha").await.unwrap();
    let id = workspace.to_string();
    let role = Role::owner_of(&id);

    // when
    let unknown = app.grant_role(owner, role, UNKNOWN_ID).await;
    let agent_owner = app.grant_role(owner, role, &agent.to_string()).await;
    let not_an_id = app.grant_role(owner, role, "nope").await;
    let on_project = app
        .grant_role(
            owner,
            Role::on("project", text(&project.id), OWNER),
            &owner.to_string(),
        )
        .await;

    // then
    assert_code(unknown, Code::NotFound);
    assert_invalid_field(&agent_owner.unwrap_err(), "role");
    assert_invalid_field(&not_an_id.unwrap_err(), "actor_id");
    assert_invalid_field(&on_project.unwrap_err(), "role");
    assert_eq!(app.owner_count(workspace).await, 1);
}

/// The role is in the list of the roles, with each permission of a workspace, and the holders tell
/// the owners. A list for the instance counts the owners of each workspace.
#[sqlx::test(migrations = false)]
async fn list_roles_shows_the_role_owner(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, owner, bob) = (app.admin().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    app.grant_role(owner, Role::owner_of(&workspace), &bob.to_string())
        .await
        .unwrap();

    // when
    let in_workspace = app.list_roles(owner, Some(&workspace)).await.unwrap();
    let on_instance = app.list_roles(admin, None).await.unwrap();
    let holders = app
        .role_holders(owner, OWNER, Some(&workspace))
        .await
        .unwrap();
    let everywhere = app.role_holders(admin, OWNER, None).await.unwrap();

    // then
    let find = |roles: &[pb::Role]| roles.iter().find(|role| role.key == OWNER).unwrap().clone();
    let role = find(&in_workspace);
    assert!(role.builtin && role.full_control);
    assert_eq!(role.grant_name, OWNER);
    assert_eq!(role.level, "workspace");
    assert_eq!(role.holder, "people");
    assert_eq!(role.permissions.len(), 35);
    assert!(role.permissions.contains(&"workspace.delete".to_owned()));
    assert_eq!(role.holder_count, 2);
    assert_eq!(holders.total, 2);
    assert_eq!(find(&on_instance).holder_count, everywhere.total);
    assert!(everywhere.total >= 2);
}

/// The holders of a custom role do not move to the role owner.
#[sqlx::test(migrations = false)]
async fn delete_role_does_not_move_to_the_role_owner(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let role = every_permission(&app, owner, &workspace).await;

    // when
    let moved = app.delete_role_moving(owner, &role, OWNER).await;

    // then
    assert_invalid_field(&moved.unwrap_err(), "move_to");
}

/// Two owners that take each other out at the same time leave one owner: the lock on the
/// workspace makes the removals run one after the other. Ten rounds, so that a race shows.
#[sqlx::test(migrations = false)]
async fn two_owners_cannot_take_each_other_out(pool: PgPool) {
    // given
    let app = app(pool).await;
    let mut rounds = Vec::new();
    for _ in 0..10 {
        let (alice, bob) = (app.user().await, app.user().await);
        let workspace = app.workspace_of(alice).await;
        let id = workspace.to_string();
        app.grant_role(alice, Role::owner_of(&id), &bob.to_string())
            .await
            .unwrap();
        rounds.push((alice, bob, workspace));
    }

    // when
    let mut passed = Vec::new();
    for (alice, bob, workspace) in &rounds {
        let (id, alice_id, bob_id) = (workspace.to_string(), alice.to_string(), bob.to_string());
        let role = Role::owner_of(&id);
        let (by_alice, by_bob) = tokio::join!(
            app.revoke_role(*alice, role, &bob_id),
            app.revoke_role(*bob, role, &alice_id),
        );
        passed.push(usize::from(by_alice.is_ok()) + usize::from(by_bob.is_ok()));
    }

    // then
    assert!(passed.iter().all(|&count| count == 1), "{passed:?}");
    for (_, _, workspace) in &rounds {
        assert_eq!(app.owner_count(*workspace).await, 1);
    }
}
