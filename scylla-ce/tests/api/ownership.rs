//! The tests of ownership: the owner of a workspace needs no role and no SpiceDB, and a role in
//! a workspace goes down to its projects.

use scylla_domain::workspaces::{Workspace, WorkspaceId};
use scylla_proto::projects::v1::CreateProjectRequest;
use scylla_testkit::{id, text, with_token, TestApp};
use sqlx::PgPool;
use tonic::Code;

use crate::agents::agent_in;
use crate::{app, assert_code};

/// Makes a creation request in a workspace.
fn create_in(workspace: WorkspaceId) -> CreateProjectRequest {
    CreateProjectRequest {
        workspace_id: Some(id(workspace.to_string())),
        name: "by agent".to_owned(),
    }
}

/// A role in a workspace goes down to each project in it, also to a project created later.
#[sqlx::test(migrations = false)]
async fn a_role_in_a_workspace_goes_down_to_its_projects(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let member = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let before = app.create_project(owner, "before").await.unwrap();
    app.grant(&[Workspace::MEMBER.of(workspace, member)]).await;
    let after = app.create_project(owner, "after").await.unwrap();

    // when
    let read_before = app.get_project(member, text(&before.id)).await;
    let read_after = app.get_project(member, text(&after.id)).await;
    let listed = app.visible_projects(member, None).await.unwrap();

    // then
    assert!(read_before.is_ok(), "{read_before:?}");
    assert!(read_after.is_ok(), "{read_after:?}");
    assert_eq!(listed, ["before", "after"]);
}

/// An agent acts with the role that an admin gives it, and only in its workspace.
#[sqlx::test(migrations = false)]
async fn an_agent_acts_with_its_role(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let other = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let elsewhere = app.workspace_of(other).await;
    let (agent, token) = agent_in(&app, owner, workspace).await;
    let mut client = app.projects.clone();

    // when
    let without_role = client
        .create_project(with_token(&token, create_in(workspace)))
        .await;
    app.grant(&[Workspace::MEMBER.of(workspace, agent)]).await;
    let with_role = client
        .create_project(with_token(&token, create_in(workspace)))
        .await;
    let in_other = client
        .create_project(with_token(&token, create_in(elsewhere)))
        .await;

    // then
    assert_code(without_role, Code::PermissionDenied);
    assert!(with_role.is_ok(), "{with_role:?}");
    assert_code(in_other, Code::PermissionDenied);
}

/// The owner of a workspace creates, lists, reads and deletes its projects with no SpiceDB. A
/// member needs SpiceDB.
#[sqlx::test(migrations = false)]
async fn an_owner_needs_no_spicedb(pool: PgPool) {
    // given
    let app = TestApp::spawn_without_spicedb(pool, scylla_ce::edition()).await;
    let owner = app.user().await;
    let member = app.user().await;
    let workspace = app.workspace_of(owner).await;

    // when
    let created = app.create_project(owner, "alpha").await;
    let listed = app.visible_projects(owner, None).await;
    let project_id = created
        .as_ref()
        .map(|project| text(&project.id).to_owned())
        .unwrap_or_default();
    let read = app.get_project(owner, &project_id).await;
    let deleted = app.delete_project(owner, &project_id).await;
    let by_member = app.create_project_in(member, workspace, "beta").await;

    // then
    assert!(created.is_ok(), "{created:?}");
    assert_eq!(listed.unwrap(), ["alpha"]);
    assert!(read.is_ok(), "{read:?}");
    assert!(deleted.is_ok(), "{deleted:?}");
    assert_code(by_member, Code::Unavailable);
}
