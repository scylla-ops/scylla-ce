//! The tests of `WorkspaceService`: data, validation and permissions.

use scylla_domain::tenancy::grants::OWNER;
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_proto::workspaces::v1::workspace_service_client::WorkspaceServiceClient;
use scylla_proto::workspaces::v1::CreateWorkspaceRequest;
use scylla_testkit::{text, with_token};
use sqlx::PgPool;
use tonic::Code;

use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// A created workspace comes back trimmed, and the caller is its only owner.
#[sqlx::test(migrations = false)]
async fn create_workspace_returns_the_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;

    // when
    let workspace = app.create_workspace(user, "  team  ").await.unwrap();
    let holders = app
        .role_holders(user, OWNER, Some(text(&workspace.id)))
        .await
        .unwrap();

    // then
    assert_eq!(workspace.name, "team");
    assert_eq!(holders.total, 1);
}

/// Only a user owns a workspace: an agent cannot create one.
#[sqlx::test(migrations = false)]
async fn create_workspace_needs_a_user(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let (_, token) = app.app_in(user, app.workspace_of(user).await, true).await;
    let request = CreateWorkspaceRequest {
        name: "bots".to_owned(),
        slug: None,
    };

    // when
    let result = WorkspaceServiceClient::new(app.channel.clone())
        .create_workspace(with_token(&token, request))
        .await;

    // then
    assert_code(result, Code::PermissionDenied);
}

/// A name of `MAX_LEN` characters passes, one more fails, and an empty name fails.
#[sqlx::test(migrations = false)]
async fn create_workspace_checks_the_name(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;

    // when
    let longest = app.create_workspace(user, &"x".repeat(100)).await;
    let too_long = app.create_workspace(user, &"x".repeat(101)).await;
    let empty = app.create_workspace(user, "   ").await;

    // then
    assert!(longest.is_ok());
    assert_invalid_field(&too_long.unwrap_err(), "name");
    assert_invalid_field(&empty.unwrap_err(), "name");
}

/// The owner and a viewer read a workspace. Another user and an unknown workspace get the same
/// refusal.
#[sqlx::test(migrations = false)]
async fn get_workspace_needs_a_role(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, viewer, stranger) = (app.user().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[Workspace::VIEWER.of(workspace, viewer)]).await;
    let id = workspace.to_string();

    // when
    let by_owner = app.get_workspace(owner, &id).await;
    let by_viewer = app.get_workspace(viewer, &id).await;
    let by_stranger = app.get_workspace(stranger, &id).await;
    let unknown = app.get_workspace(owner, UNKNOWN_ID).await;
    let not_an_id = app.get_workspace(owner, "nope").await;

    // then
    assert!(by_owner.is_ok() && by_viewer.is_ok());
    assert_code(by_stranger, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
    assert_invalid_field(&not_an_id.unwrap_err(), "workspace_id");
}

/// The list shows the workspaces that the caller owns and the workspaces where it has a role,
/// oldest first.
#[sqlx::test(migrations = false)]
async fn list_workspaces_shows_owned_and_member_workspaces(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (alice, bob, carol) = (app.user().await, app.user().await, app.user().await);
    let team = app.create_workspace(alice, "team").await.unwrap();
    app.create_workspace(carol, "private").await.unwrap();
    let bob_workspace = app.workspace_of(bob).await;
    app.grant(&[Workspace::MEMBER.of(bob_workspace, alice)])
        .await;

    // when
    let listed = app.visible_workspaces(alice).await.unwrap();

    // then
    assert_eq!(listed, ["Personal", "Personal", "team"]);
    assert_eq!(team.name, "team");
}

/// A new workspace has the time zone UTC. An admin sets an IANA name, a reader gets it, and a
/// change of another field keeps it. A member gets a refusal.
#[sqlx::test(migrations = false)]
async fn update_workspace_sets_the_time_zone(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, admin, member) = (app.user().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[
        Workspace::ADMIN.of(workspace, admin),
        Workspace::MEMBER.of(workspace, member),
    ])
    .await;
    let id = workspace.to_string();
    let before = app.get_workspace(owner, &id).await.unwrap();

    // when
    let changed = app
        .set_workspace_time_zone(admin, &id, " Europe/Paris ")
        .await;
    let renamed = app.update_workspace(owner, &id, Some("Home"), None).await;
    let by_member = app.set_workspace_time_zone(member, &id, "Asia/Tokyo").await;
    let read = app.get_workspace(member, &id).await.unwrap();

    // then
    assert_eq!(before.time_zone, "UTC");
    assert_eq!(changed.unwrap().time_zone, "Europe/Paris");
    assert_eq!(renamed.unwrap().time_zone, "Europe/Paris");
    assert_code(by_member, Code::PermissionDenied);
    assert_eq!(read.time_zone, "Europe/Paris");
}

/// A time zone is a name of the IANA database, in its case. A link of the database passes. An
/// unknown name, an abbreviation, an offset and an empty text give `INVALID_ARGUMENT` on
/// `time_zone`, and the zone does not change.
#[sqlx::test(migrations = false)]
async fn update_workspace_checks_the_time_zone(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let id = app.workspace_of(owner).await.to_string();

    // when
    let link = app.set_workspace_time_zone(owner, &id, "US/Eastern").await;
    let refused = [
        app.set_workspace_time_zone(owner, &id, "Mars/Olympus")
            .await,
        app.set_workspace_time_zone(owner, &id, "europe/paris")
            .await,
        app.set_workspace_time_zone(owner, &id, "PST").await,
        app.set_workspace_time_zone(owner, &id, "+02:00").await,
        app.set_workspace_time_zone(owner, &id, "").await,
    ];
    let read = app.get_workspace(owner, &id).await.unwrap();

    // then
    assert_eq!(link.unwrap().time_zone, "US/Eastern");
    for result in refused {
        assert_invalid_field(&result.unwrap_err(), "time_zone");
    }
    assert_eq!(read.time_zone, "US/Eastern");
}
