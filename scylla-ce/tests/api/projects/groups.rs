//! The tests of `ProjectGroupService`: the groups of the projects of a workspace, their order, the
//! moves of the projects, and the roles that reach them.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::projects::v1::{ListProjectsRequest, ProjectOrder, ProjectSection};
use scylla_testkit::{id, text, TestApp};
use sqlx::PgPool;
use tonic::Code;

use super::cast::{passed, Cast};
use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// Returns the names of the projects of a workspace in the order of the workspace, of one
/// section or of all.
async fn ordered(
    app: &TestApp,
    actor: ActorId,
    workspace: WorkspaceId,
    section: Option<ProjectSection>,
) -> Vec<String> {
    let request = ListProjectsRequest {
        workspace_id: Some(id(workspace.to_string())),
        order: ProjectOrder::Position.into(),
        section,
        ..ListProjectsRequest::default()
    };
    let list = app.search_projects(actor, request).await.unwrap();
    list.projects
        .into_iter()
        .map(|project| project.name)
        .collect()
}

/// Makes projects in the personal workspace of an actor, and returns their ids in order.
async fn projects(app: &TestApp, actor: ActorId, names: &[&str]) -> Vec<String> {
    let mut ids = Vec::new();
    for name in names {
        let project = app.create_project(actor, name).await.unwrap();
        ids.push(text(&project.id).to_owned());
    }
    ids
}

/// A new group comes after the other groups, with a trimmed name, and the list gives the groups
/// in their order.
#[sqlx::test(migrations = false)]
async fn create_project_group_puts_the_group_last(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;

    // when
    let first = app
        .create_project_group(owner, workspace, "  Deploys  ")
        .await;
    let second = app.create_project_group(owner, workspace, "CI").await;
    let groups = app.list_project_groups(owner, workspace).await.unwrap();

    // then
    let first = first.unwrap();
    assert_eq!(first.name, "Deploys");
    assert_eq!(text(&first.workspace_id), workspace.to_string());
    let names: Vec<_> = groups.iter().map(|group| group.name.as_str()).collect();
    assert_eq!(names, ["Deploys", "CI"]);
    assert_eq!(groups[1], second.unwrap());
}

/// An empty name or a name of more than 100 characters gives `INVALID_ARGUMENT` on the name.
#[sqlx::test(migrations = false)]
async fn create_project_group_refuses_a_bad_name(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;

    // when
    let empty = app.create_project_group(owner, workspace, "   ").await;
    let long = app
        .create_project_group(owner, workspace, &"x".repeat(101))
        .await;

    // then
    assert_invalid_field(&empty.unwrap_err(), "name");
    assert_invalid_field(&long.unwrap_err(), "name");
    assert!(app
        .list_project_groups(owner, workspace)
        .await
        .unwrap()
        .is_empty());
}

/// A new place moves the group in the order of the groups, a place after the end puts it last,
/// and a new name changes only the name.
#[sqlx::test(migrations = false)]
async fn update_project_group_renames_and_moves_the_group(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let mut ids = Vec::new();
    for name in ["a", "b", "c"] {
        let group = app.create_project_group(owner, workspace, name).await;
        ids.push(text(&group.unwrap().id).to_owned());
    }

    // when
    let moved = app
        .update_project_group(owner, &ids[2], None, Some(0))
        .await;
    let renamed = app
        .update_project_group(owner, &ids[0], Some(" alpha "), Some(99))
        .await;
    let groups = app.list_project_groups(owner, workspace).await.unwrap();

    // then
    assert_eq!(moved.unwrap().name, "c");
    assert_eq!(renamed.unwrap().name, "alpha");
    let names: Vec<_> = groups.iter().map(|group| group.name.as_str()).collect();
    assert_eq!(names, ["c", "b", "alpha"]);
}

/// A move puts a project in a group before another project, or last, and the order of the
/// workspace gives the groups first, then the projects with no group.
#[sqlx::test(migrations = false)]
async fn move_project_orders_the_projects_of_a_group(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let ids = projects(&app, owner, &["a", "b", "c", "d"]).await;
    let group = app.create_project_group(owner, workspace, "G").await;
    let group = text(&group.unwrap().id).to_owned();

    // when
    let moved = app.move_project(owner, &ids[1], Some(&group), None).await;
    app.move_project(owner, &ids[3], Some(&group), Some(&ids[1]))
        .await
        .unwrap();
    app.move_project(owner, &ids[2], None, Some(&ids[0]))
        .await
        .unwrap();

    // then
    assert_eq!(text(&moved.unwrap().group_id), group);
    let in_group = ProjectSection {
        group_id: Some(id(group.as_str())),
    };
    let loose = ProjectSection { group_id: None };
    assert_eq!(
        ordered(&app, owner, workspace, Some(in_group)).await,
        ["d", "b"]
    );
    assert_eq!(
        ordered(&app, owner, workspace, Some(loose)).await,
        ["c", "a"]
    );
    assert_eq!(
        ordered(&app, owner, workspace, None).await,
        ["d", "b", "c", "a"]
    );
}

/// A delete of a group puts its projects after the projects with no group, in their order.
#[sqlx::test(migrations = false)]
async fn delete_project_group_puts_its_projects_after_the_others(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let ids = projects(&app, owner, &["a", "b", "c", "d"]).await;
    let group = app.create_project_group(owner, workspace, "G").await;
    let group = text(&group.unwrap().id).to_owned();
    app.move_project(owner, &ids[3], Some(&group), None)
        .await
        .unwrap();
    app.move_project(owner, &ids[0], Some(&group), None)
        .await
        .unwrap();

    // when
    let deleted = app.delete_project_group(owner, &group).await;

    // then
    deleted.unwrap();
    assert!(app
        .list_project_groups(owner, workspace)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        ordered(&app, owner, workspace, None).await,
        ["b", "c", "d", "a"]
    );
    let project = app.get_project(owner, &ids[0]).await.unwrap();
    assert_eq!(project.group_id, None);
}

/// A group of another workspace and the project itself as the project to come before give
/// `INVALID_ARGUMENT`. An unknown group and an unknown project give `PERMISSION_DENIED`.
#[sqlx::test(migrations = false)]
async fn move_project_refuses_a_bad_group_or_place(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let other = app.user().await;
    let ids = projects(&app, owner, &["a"]).await;
    let foreign = app.workspace_of(other).await;
    let foreign = app.create_project_group(other, foreign, "theirs").await;
    let foreign = text(&foreign.unwrap().id).to_owned();

    // when
    let elsewhere = app.move_project(owner, &ids[0], Some(&foreign), None).await;
    let itself = app.move_project(owner, &ids[0], None, Some(&ids[0])).await;
    let unknown = app.move_project(owner, UNKNOWN_ID, None, None).await;
    let rename = app
        .update_project_group(owner, UNKNOWN_ID, Some("x"), None)
        .await;
    let delete = app.delete_project_group(owner, &foreign).await;

    // then
    assert_invalid_field(&elsewhere.unwrap_err(), "group_id");
    assert_invalid_field(&itself.unwrap_err(), "before_project_id");
    assert_code(unknown, Code::PermissionDenied);
    assert_code(rename, Code::PermissionDenied);
    assert_code(delete, Code::PermissionDenied);
}

/// The readers of the workspace list the groups. Its owners, admins and members change them. The
/// editors of a project also move it.
#[sqlx::test(migrations = false)]
async fn the_routes_of_the_groups_follow_the_roles(pool: PgPool) {
    // given
    let app = app(pool).await;
    let cast = Cast::new(&app).await;
    let workspace = app.workspace_of(cast.owner()).await;
    let group = app.create_project_group(cast.owner(), workspace, "G").await;
    let group = text(&group.unwrap().id).to_owned();
    let mut lists = Vec::new();
    let mut creates = Vec::new();
    let mut renames = Vec::new();
    let mut moves = Vec::new();

    // when
    for (role, actor) in cast.actors {
        lists.push((role, app.list_project_groups(actor, workspace).await));
        creates.push((role, app.create_project_group(actor, workspace, role).await));
        let rename = app.update_project_group(actor, &group, Some(role), None);
        renames.push((role, rename.await));
        let moved = app.move_project(actor, &cast.project, Some(&group), None);
        moves.push((role, moved.await));
    }

    // then
    let workers = ["owner", "workspace admin", "member"];
    assert_eq!(
        passed(lists),
        ["owner", "workspace admin", "member", "workspace viewer"]
    );
    assert_eq!(passed(creates), workers);
    assert_eq!(passed(renames), workers);
    assert_eq!(
        passed(moves),
        [
            "owner",
            "workspace admin",
            "member",
            "project admin",
            "editor"
        ]
    );
}
