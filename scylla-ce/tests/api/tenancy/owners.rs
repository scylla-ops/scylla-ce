//! The tests of the owners of a workspace: a workspace has one owner or more, only an owner
//! changes them, and the last owner stays.

use scylla_domain::tenancy::workspaces::Workspace;
use scylla_testkit::text;
use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::assert_refused;
use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// An owner makes a user an owner: the new owner gets each right with no role, and the list
/// shows the two owners. A second addition changes nothing.
#[sqlx::test(migrations = false)]
async fn an_added_owner_gets_each_right(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (alice, bob) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(alice).await;
    let project = app.create_project(alice, "alpha").await.unwrap();
    let (id, bob_id) = (workspace.to_string(), bob.to_string());

    // when
    let before = app.can(bob, Workspace::ADD_OWNER.on(workspace)).await;
    app.add_owner(alice, &id, &bob_id).await.unwrap();
    app.add_owner(alice, &id, &bob_id).await.unwrap();
    let after = app.can(bob, Workspace::ADD_OWNER.on(workspace)).await;
    let deleted = app.delete_project(bob, text(&project.id)).await;
    let owners = app.owners(bob, &id).await.unwrap();

    // then
    assert!(!before && after);
    assert!(deleted.is_ok());
    let mut expected = vec![alice.to_string(), bob_id];
    expected.sort();
    assert_eq!(owners, expected);
}

/// A workspace admin, a viewer and another user cannot change the owners: no role gives this
/// right. An unknown workspace gets the same refusal.
#[sqlx::test(migrations = false)]
async fn only_an_owner_changes_the_owners(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, admin, viewer) = (app.user().await, app.user().await, app.user().await);
    let stranger = app.user().await;
    let workspace = app.workspace_of(owner).await;
    app.grant(&[
        Workspace::ADMIN.of(workspace, admin),
        Workspace::VIEWER.of(workspace, viewer),
    ])
    .await;
    let (id, owner_id) = (workspace.to_string(), owner.to_string());

    // when
    let added_by_admin = app.add_owner(admin, &id, &admin.to_string()).await;
    let added_by_viewer = app.add_owner(viewer, &id, &viewer.to_string()).await;
    let removed_by_admin = app.remove_owner(admin, &id, &owner_id).await;
    let removed_by_stranger = app.remove_owner(stranger, &id, &owner_id).await;
    let unknown = app.add_owner(owner, UNKNOWN_ID, &admin.to_string()).await;

    // then
    assert_code(added_by_admin, Code::PermissionDenied);
    assert_code(added_by_viewer, Code::PermissionDenied);
    assert_code(removed_by_admin, Code::PermissionDenied);
    assert_code(removed_by_stranger, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
    assert_eq!(app.owner_count(workspace).await, 1);
}

/// An owner takes out another owner, who loses its rights at once. The last owner stays, and a
/// user that is not an owner changes nothing.
#[sqlx::test(migrations = false)]
async fn the_last_owner_stays(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (alice, bob) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(alice).await;
    let (id, alice_id, bob_id) = (workspace.to_string(), alice.to_string(), bob.to_string());
    app.add_owner(alice, &id, &bob_id).await.unwrap();

    // when
    app.remove_owner(bob, &id, &alice_id).await.unwrap();
    let alice_reads = app.can(alice, Workspace::GET_WORKSPACE.on(workspace)).await;
    let last = app.remove_owner(bob, &id, &bob_id).await;
    let not_an_owner = app.remove_owner(bob, &id, &alice_id).await;

    // then
    assert!(!alice_reads);
    assert_refused(last, "LAST_OWNER");
    assert!(not_an_owner.is_ok());
    assert_eq!(app.owners(bob, &id).await.unwrap(), [bob_id]);
}

/// Only a user can be an owner: an unknown user and an agent give `NOT_FOUND`, and a value that
/// is not an id gives `INVALID_ARGUMENT`.
#[sqlx::test(migrations = false)]
async fn add_owner_needs_a_user(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let agent = app.app_in(owner, workspace, true).await.0.actor();
    let id = workspace.to_string();

    // when
    let unknown = app.add_owner(owner, &id, UNKNOWN_ID).await;
    let agent_owner = app.add_owner(owner, &id, &agent.to_string()).await;
    let not_an_id = app.add_owner(owner, &id, "nope").await;

    // then
    assert_code(unknown, Code::NotFound);
    assert_code(agent_owner, Code::NotFound);
    assert_invalid_field(&not_an_id.unwrap_err(), "user_id");
    assert_eq!(app.owner_count(workspace).await, 1);
}

/// A reader of the workspace lists its owners. Another user and an unknown workspace get the
/// same refusal.
#[sqlx::test(migrations = false)]
async fn list_owners_needs_a_role(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, viewer, stranger) = (app.user().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[Workspace::VIEWER.of(workspace, viewer)]).await;
    let id = workspace.to_string();

    // when
    let by_viewer = app.owners(viewer, &id).await;
    let by_stranger = app.owners(stranger, &id).await;
    let unknown = app.owners(owner, UNKNOWN_ID).await;

    // then
    assert_eq!(by_viewer.unwrap(), [owner.to_string()]);
    assert_code(by_stranger, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
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
        app.add_owner(alice, &id, &bob.to_string()).await.unwrap();
        rounds.push((alice, bob, workspace));
    }

    // when
    let mut passed = Vec::new();
    for (alice, bob, workspace) in &rounds {
        let (id, alice_id, bob_id) = (workspace.to_string(), alice.to_string(), bob.to_string());
        let (by_alice, by_bob) = tokio::join!(
            app.remove_owner(*alice, &id, &bob_id),
            app.remove_owner(*bob, &id, &alice_id),
        );
        passed.push(usize::from(by_alice.is_ok()) + usize::from(by_bob.is_ok()));
    }

    // then
    assert!(passed.iter().all(|&count| count == 1), "{passed:?}");
    for (_, _, workspace) in &rounds {
        assert_eq!(app.owner_count(*workspace).await, 1);
    }
}
