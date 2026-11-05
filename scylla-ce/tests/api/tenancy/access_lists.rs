//! The tests of `ListAccess`: the owners and the roles of a workspace, the roles on a project,
//! the pages, and who can read the list.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_testkit::{text, Role};
use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::PASSWORD;
use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// The list of a workspace has its owners first, in user id order, then each role, with the
/// kind of each actor and the display name and the handle of a user.
#[sqlx::test(migrations = false)]
async fn list_access_shows_the_owners_and_the_roles(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, partner, member) = (app.user().await, app.user().await, app.user().await);
    let admin = app.admin().await;
    let carol = app
        .create_user(admin, "carol@example.com", "carol", PASSWORD)
        .await
        .unwrap();
    let carol_id = text(&carol.id).to_owned();
    let workspace = app.workspace_of(owner).await;
    let id = workspace.to_string();
    let agent = app.app_in(owner, workspace, true).await.0.actor();
    app.add_owner(owner, &id, &partner.to_string())
        .await
        .unwrap();
    for (role, to) in [("admin", carol_id.clone()), ("member", member.to_string())] {
        let role = Role::on("workspace", &id, role);
        app.grant_role(owner, role, &to).await.unwrap();
    }
    app.grant_role(
        owner,
        Role::on("workspace", &id, "viewer"),
        &agent.to_string(),
    )
    .await
    .unwrap();

    // when
    let page = app
        .list_access(member, "workspace", &id, 100, "")
        .await
        .unwrap();

    // then
    let mut owners = [owner.to_string(), partner.to_string()];
    owners.sort();
    let entries = page.entries;
    let listed: Vec<_> = entries
        .iter()
        .map(|entry| (text(&entry.actor_id).to_owned(), entry.role.as_str()))
        .collect();
    assert_eq!(
        listed[..2],
        [(owners[0].clone(), "owner"), (owners[1].clone(), "owner")]
    );
    let mut roles = listed[2..].to_vec();
    roles.sort_by(|a, b| a.1.cmp(b.1));
    let expected = [
        (carol_id.clone(), "admin"),
        (member.to_string(), "member"),
        (agent.to_string(), "viewer"),
    ];
    assert_eq!(roles, expected);
    let carol = entries
        .iter()
        .find(|entry| text(&entry.actor_id) == carol_id);
    let carol = carol.unwrap();
    let robot = entries.iter().find(|entry| entry.role == "viewer").unwrap();
    assert_eq!(
        (
            carol.actor_kind.as_str(),
            carol.display_name.as_str(),
            carol.handle.as_str()
        ),
        ("user", "Created User", "carol")
    );
    assert_eq!(
        (robot.actor_kind.as_str(), robot.display_name.as_str()),
        ("app", "build machine")
    );
    assert!(page.page.unwrap().next_token.is_empty());
}

/// The pages of the list give each entry one time, in the same order, at each page size.
#[sqlx::test(migrations = false)]
async fn list_access_gives_each_entry_in_pages(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let id = workspace.to_string();
    app.add_owner(owner, &id, &app.user().await.to_string())
        .await
        .unwrap();
    for role in ["admin", "member", "viewer"] {
        let to = app.user().await.to_string();
        app.grant_role(owner, Role::on("workspace", &id, role), &to)
            .await
            .unwrap();
    }

    // when
    let whole = app.access_of(owner, "workspace", &id, 100).await.unwrap();
    let by_one = app.access_of(owner, "workspace", &id, 1).await.unwrap();
    let by_two = app.access_of(owner, "workspace", &id, 2).await.unwrap();
    let first = app
        .list_access(owner, "workspace", &id, 2, "")
        .await
        .unwrap();

    // then
    assert_eq!(whole.len(), 5);
    assert_eq!(by_one, whole);
    assert_eq!(by_two, whole);
    assert_eq!(first.entries.len(), 2);
    assert!(!first.page.unwrap().next_token.is_empty());
}

/// The list of a project has the roles on the project, not the owners of the workspace.
#[sqlx::test(migrations = false)]
async fn list_access_shows_the_roles_on_a_project(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let (editor, viewer) = (app.user().await, app.user().await);
    let project = app.create_project(owner, "alpha").await.unwrap();
    let id = text(&project.id);
    for (role, to) in [("editor", editor), ("viewer", viewer)] {
        app.join_workspace(owner, to).await;
        let role = Role::on("project", id, role);
        app.grant_role(owner, role, &to.to_string()).await.unwrap();
    }

    // when
    let mut listed = app.access_of(viewer, "project", id, 100).await.unwrap();

    // then
    listed.sort_by(|a, b| a.1.cmp(&b.1));
    let expected = [
        (editor.to_string(), "editor".to_owned()),
        (viewer.to_string(), "viewer".to_owned()),
    ];
    assert_eq!(listed, expected);
}

/// A reader of a workspace lists its access, and a viewer of a project or a reader of its
/// workspace lists the access of the project. Another user and an unknown object get the same
/// refusal. A kind with no roles and a token that the list did not make are request errors.
#[sqlx::test(migrations = false)]
async fn list_access_needs_a_reader(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let (reader, viewer, stranger) = (app.user().await, app.user().await, app.user().await);
    let project = app.create_project(owner, "alpha").await.unwrap();
    let project_id = ProjectId::parse(text(&project.id)).unwrap();
    let workspace = WorkspaceId::parse(text(&project.workspace_id)).unwrap();
    app.grant(&[
        Workspace::VIEWER.of(workspace, reader),
        Project::VIEWER.of(project_id, viewer),
    ])
    .await;
    let (w, p) = (workspace.to_string(), project_id.to_string());

    // when
    let reads_workspace = app.list_access(reader, "workspace", &w, 10, "").await;
    let reads_project = app.list_access(reader, "project", &p, 10, "").await;
    let viewer_on_project = app.list_access(viewer, "project", &p, 10, "").await;
    let viewer_on_workspace = app.list_access(viewer, "workspace", &w, 10, "").await;
    let by_stranger = app.list_access(stranger, "project", &p, 10, "").await;
    let unknown = app.list_access(owner, "project", UNKNOWN_ID, 10, "").await;
    let bad_kind = app.list_access(owner, "platform", UNKNOWN_ID, 10, "").await;
    let bad_token = app.list_access(owner, "workspace", &w, 10, "nope").await;
    let owners_of_project = app
        .list_access(
            owner,
            "project",
            &p,
            10,
            &format!("owners.{}", ActorId::generate()),
        )
        .await;

    // then
    assert!(reads_workspace.is_ok() && reads_project.is_ok());
    assert!(viewer_on_project.is_ok(), "{viewer_on_project:?}");
    assert_code(viewer_on_workspace, Code::PermissionDenied);
    assert_code(by_stranger, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
    assert_invalid_field(&bad_kind.unwrap_err(), "object.kind");
    assert_invalid_field(&bad_token.unwrap_err(), "page.token");
    assert_invalid_field(&owners_of_project.unwrap_err(), "page.token");
}
