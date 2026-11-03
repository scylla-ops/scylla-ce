//! The tests of `CheckPermissions`: the answers for each kind of caller, an unknown object, and
//! the names that are not permissions.

use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_testkit::text;
use sqlx::PgPool;

use crate::{app, assert_invalid_field, UNKNOWN_ID};

/// The permissions that the tests ask for on a workspace: a read, a work, a manage and an owner
/// route.
const ON_WORKSPACE: [&str; 4] = [
    "get_workspace",
    "create_project",
    "update_workspace",
    "delete_workspace",
];

/// Each caller gets the permissions of its rights in a workspace: an owner each one, an admin
/// all but the routes of the owners, a member the work, a viewer the reads, and another user
/// and an instance admin none.
#[sqlx::test(migrations = false)]
async fn check_permissions_answers_for_each_role(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let (admin, member, viewer) = (app.user().await, app.user().await, app.user().await);
    let (stranger, instance_admin) = (app.user().await, app.admin().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[
        Workspace::ADMIN.of(workspace, admin),
        Workspace::MEMBER.of(workspace, member),
        Workspace::VIEWER.of(workspace, viewer),
    ])
    .await;
    let id = workspace.to_string();

    // when
    let mut answers = Vec::new();
    for caller in [owner, admin, member, viewer, stranger, instance_admin] {
        let answer = app.check_permissions(caller, "workspace", &id, &ON_WORKSPACE);
        answers.push(answer.await.unwrap());
    }

    // then
    let expected = [
        [true, true, true, true],
        [true, true, true, false],
        [true, true, false, false],
        [true, false, false, false],
        [false, false, false, false],
        [false, false, false, false],
    ];
    assert_eq!(answers, expected);
}

/// A role on one project gives its permissions on that project only, and a role in the
/// workspace gives them on each project of it.
#[sqlx::test(migrations = false)]
async fn check_permissions_sees_the_roles_on_a_project(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let (editor, reader) = (app.user().await, app.user().await);
    let project = app.create_project(owner, "alpha").await.unwrap();
    let id = text(&project.id);
    let workspace = WorkspaceId::parse(text(&project.workspace_id)).unwrap();
    app.grant(&[
        Project::EDITOR.of(ProjectId::parse(id).unwrap(), editor),
        Workspace::VIEWER.of(workspace, reader),
    ])
    .await;
    let names = ["get_project", "delete_project", "list_access"];

    // when
    let by_owner = app.check_permissions(owner, "project", id, &names).await;
    let by_editor = app.check_permissions(editor, "project", id, &names).await;
    let by_reader = app.check_permissions(reader, "project", id, &names).await;

    // then
    assert_eq!(by_owner.unwrap(), [true, true, true]);
    assert_eq!(by_editor.unwrap(), [true, false, true]);
    assert_eq!(by_reader.unwrap(), [true, false, true]);
}

/// An instance admin has each permission on the platform, and a user none.
#[sqlx::test(migrations = false)]
async fn check_permissions_answers_on_the_platform(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, user) = (app.admin().await, app.user().await);
    let names = ["list_users", "delete_user"];

    // when
    let by_admin = app
        .check_permissions(admin, "platform", UNKNOWN_ID, &names)
        .await;
    let by_user = app
        .check_permissions(user, "platform", UNKNOWN_ID, &names)
        .await;

    // then
    assert_eq!(by_admin.unwrap(), [true, true]);
    assert_eq!(by_user.unwrap(), [false, false]);
}

/// An unknown object gives `false` for each permission. A name that is not a permission of the
/// definition, a relation, too many names, a kind with no permissions to check and a bad id
/// are request errors.
#[sqlx::test(migrations = false)]
async fn check_permissions_needs_known_names(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let id = app.workspace_of(owner).await.to_string();
    let many = vec!["get_workspace"; 51];

    // when
    let unknown = app
        .check_permissions(owner, "workspace", UNKNOWN_ID, &["get_workspace"])
        .await;
    let fifty = app
        .check_permissions(owner, "workspace", &id, &many[..50])
        .await;
    let not_a_name = app
        .check_permissions(owner, "workspace", &id, &["get_workspace", "fly"])
        .await;
    let a_relation = app
        .check_permissions(owner, "workspace", &id, &["admin"])
        .await;
    let other_kind = app
        .check_permissions(owner, "workspace", &id, &["delete_user"])
        .await;
    let too_many = app.check_permissions(owner, "workspace", &id, &many).await;
    let bad_kind = app.check_permissions(owner, "actor", &id, &["x"]).await;
    let bad_id = app.check_permissions(owner, "workspace", "nope", &[]).await;

    // then
    assert_eq!(unknown.unwrap(), [false]);
    assert_eq!(fifty.unwrap(), vec![true; 50]);
    for result in [not_a_name, a_relation, other_kind, too_many] {
        assert_invalid_field(&result.unwrap_err(), "permissions");
    }
    assert_invalid_field(&bad_kind.unwrap_err(), "object.kind");
    assert_invalid_field(&bad_id.unwrap_err(), "object.id");
}
