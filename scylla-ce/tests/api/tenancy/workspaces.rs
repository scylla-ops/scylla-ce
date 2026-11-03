//! The tests of `WorkspaceService`: data, validation and permissions.

use scylla_domain::tenancy::workspaces::Workspace;
use scylla_proto::workspaces::v1::workspace_service_client::WorkspaceServiceClient;
use scylla_proto::workspaces::v1::CreateWorkspaceRequest;
use scylla_testkit::{text, with_token};
use sqlx::PgPool;
use tonic::Code;

use crate::identity::agents::agent_in;
use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// A created workspace comes back trimmed, and the caller is its only owner.
#[sqlx::test(migrations = false)]
async fn create_workspace_returns_the_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;

    // when
    let workspace = app.create_workspace(user, "  team  ").await.unwrap();
    let owners = app.owners(user, text(&workspace.id)).await.unwrap();

    // then
    assert_eq!(workspace.name, "team");
    assert_eq!(owners, [user.to_string()]);
}

/// Only a user owns a workspace: an agent cannot create one.
#[sqlx::test(migrations = false)]
async fn create_workspace_needs_a_user(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let (_, token) = agent_in(&app, user, app.workspace_of(user).await).await;
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
