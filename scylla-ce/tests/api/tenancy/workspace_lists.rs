//! The tests of the lists of workspaces: the details of each workspace, the own workspaces of an
//! instance admin, and the console of the workspaces of the instance.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_proto::common::v1::PageRequest;
use scylla_proto::workspaces::v1::{ListAllWorkspacesRequest, WorkspaceOrder};
use scylla_testkit::{text, RoleInput, TestApp};
use sqlx::PgPool;
use tonic::Code;

use crate::projects::icons::glyph;
use crate::{app, assert_code, assert_invalid_field};

/// Gives a user a display name and a handle, with no route.
async fn name_user(app: &TestApp, user: ActorId, name: &str, handle: &str) {
    sqlx::query("UPDATE users SET display_name = $2, handle = $3 WHERE id = $1")
        .bind(user.to_db())
        .bind(name)
        .bind(handle)
        .execute(&app.pool)
        .await
        .expect("name the user");
}

/// Makes a request of the console with a search text, an order and a page.
fn search(query: &str, order: WorkspaceOrder, size: i32, token: &str) -> ListAllWorkspacesRequest {
    ListAllWorkspacesRequest {
        query: query.to_owned(),
        order: order.into(),
        others_only: false,
        page: Some(PageRequest {
            size,
            token: token.to_owned(),
        }),
    }
}

/// Returns the display names of the owners of the workspaces of an answer.
fn owners(details: &[scylla_proto::workspaces::v1::WorkspaceDetails]) -> Vec<String> {
    details
        .iter()
        .map(|one| one.owner.clone().expect("an owner").display_name)
        .collect()
}

/// Each workspace of the list has its first owner, its counts and the role of the caller. A
/// member does not get the email of the owner.
#[sqlx::test(migrations = false)]
async fn list_workspaces_gives_the_details_of_each_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (alice, bob, carol) = (app.user().await, app.user().await, app.user().await);
    name_user(&app, alice, "Alice Martin", "alice").await;
    let team = app.create_workspace(alice, "team").await.unwrap();
    let team_id = WorkspaceId::parse(text(&team.id)).unwrap();
    app.grant(&[
        Workspace::MEMBER.of(team_id, bob),
        Workspace::VIEWER.of(team_id, carol),
    ])
    .await;
    app.create_project_in(alice, team_id, "web").await.unwrap();

    // when
    let listed = app.list_workspaces(bob, false).await.unwrap();
    let read = app
        .get_workspace_with_details(bob, text(&team.id))
        .await
        .unwrap();

    // then
    let names: Vec<&str> = listed
        .workspaces
        .iter()
        .map(|one| one.name.as_str())
        .collect();
    assert_eq!(names, ["Personal", "team"]);
    let (own, shared) = (&listed.details[0], &listed.details[1]);
    assert_eq!(
        (own.role.as_str(), own.member_count, own.project_count),
        ("owner", 1, 0)
    );
    assert_eq!(shared.workspace_id, team.id);
    assert_eq!(
        (
            shared.role.as_str(),
            shared.member_count,
            shared.project_count
        ),
        ("member", 3, 1)
    );
    let owner = shared.owner.clone().expect("an owner");
    assert_eq!(
        (owner.display_name.as_str(), owner.handle.as_str()),
        ("Alice Martin", "alice")
    );
    assert_eq!(owner.email, "");
    assert_eq!(read.details.expect("details").role, "member");
}

/// An instance admin sees each workspace, with the emails of the owners and no role in the
/// workspaces of the others. With `only_with_role`, it sees only its own.
#[sqlx::test(migrations = false)]
async fn list_workspaces_with_only_with_role_keeps_the_own_workspaces(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, user) = (app.admin().await, app.user().await);
    app.create_workspace(admin, "ops").await.unwrap();

    // when
    let all = app.list_workspaces(admin, false).await.unwrap();
    let own = app.list_workspaces(admin, true).await.unwrap();
    let by_user = app.list_workspaces(user, true).await.unwrap();

    // then
    assert_eq!(all.workspaces.len(), 3);
    let other = all
        .details
        .iter()
        .find(|one| one.role.is_empty())
        .expect("the workspace of the user");
    assert!(other
        .owner
        .clone()
        .expect("an owner")
        .email
        .ends_with("@test.example"));
    let names: Vec<&str> = own.workspaces.iter().map(|one| one.name.as_str()).collect();
    assert_eq!(names, ["Personal", "ops"]);
    assert_eq!(by_user.workspaces.len(), 1);
}

/// The search of the console finds a workspace by the name, the handle or the email of its owner,
/// and a text over 100 characters fails.
#[sqlx::test(migrations = false)]
async fn list_all_workspaces_finds_a_workspace_by_its_owner(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let alice = app.user_with("alice.martin@example.test", None).await;
    let bob = app.user().await;
    name_user(&app, alice, "Alice Martin", "amartin").await;
    name_user(&app, bob, "Bob Stone", "bstone").await;
    let name = WorkspaceOrder::Name;

    // when
    let by_name = app
        .list_all_workspaces(admin, search(" MARTIN ", name, 0, ""))
        .await;
    let by_handle = app
        .list_all_workspaces(admin, search("bston", name, 0, ""))
        .await;
    let by_email = app
        .list_all_workspaces(admin, search("@example.test", name, 0, ""))
        .await;
    let none = app
        .list_all_workspaces(admin, search("nobody", name, 0, ""))
        .await;
    let too_long = app
        .list_all_workspaces(admin, search(&"a".repeat(101), name, 0, ""))
        .await;

    // then
    let by_name = by_name.unwrap();
    assert_eq!(
        (owners(&by_name.details), by_name.total),
        (vec!["Alice Martin".to_owned()], 1)
    );
    assert_eq!(
        by_name.details[0].owner.clone().unwrap().email,
        "alice.martin@example.test"
    );
    assert_eq!(owners(&by_handle.unwrap().details), ["Bob Stone"]);
    assert_eq!(owners(&by_email.unwrap().details), ["Alice Martin"]);
    assert_eq!(none.unwrap().total, 0);
    assert_invalid_field(&too_long.unwrap_err(), "query");
}

/// The console gives pages in the order of the request, with a token and the total on each
/// page: by name then owner, and newest first.
#[sqlx::test(migrations = false)]
async fn list_all_workspaces_gives_pages_with_a_token(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    name_user(&app, admin, "Zoe Admin", "zoe").await;
    for name in ["Carl", "Anna", "Bea", "Dan"] {
        let user = app.user().await;
        name_user(&app, user, name, &name.to_lowercase()).await;
    }
    let (by_name, newest) = (WorkspaceOrder::Name, WorkspaceOrder::Newest);

    // when
    let first = app
        .list_all_workspaces(admin, search("", by_name, 2, ""))
        .await
        .unwrap();
    let token = first.page.clone().unwrap().next_token;
    let second = app
        .list_all_workspaces(admin, search("", by_name, 2, &token))
        .await
        .unwrap();
    let token = second.page.clone().unwrap().next_token;
    let last = app
        .list_all_workspaces(admin, search("", by_name, 2, &token))
        .await
        .unwrap();
    let recent = app
        .list_all_workspaces(admin, search("", newest, 2, ""))
        .await
        .unwrap();
    let bad = app
        .list_all_workspaces(admin, search("", by_name, 2, "x"))
        .await;

    // then
    assert_eq!(owners(&first.details), ["Anna", "Bea"]);
    assert_eq!(owners(&second.details), ["Carl", "Dan"]);
    assert_eq!(owners(&last.details), ["Zoe Admin"]);
    assert_eq!(last.page.unwrap().next_token, "");
    assert_eq!([first.total, second.total, last.total], [5, 5, 5]);
    assert_eq!(owners(&recent.details), ["Dan", "Bea"]);
    assert_code(bad, Code::InvalidArgument);
}

/// Each workspace of the console comes with its icon, as the other reads give it.
#[sqlx::test(migrations = false)]
async fn list_all_workspaces_gives_the_icon_of_each_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, alice) = (app.admin().await, app.user().await);
    let team = app.create_workspace(alice, "team").await.unwrap();
    let rocket = glyph("rocket", "#a1b2c3");
    let team = app.set_workspace_icon(alice, text(&team.id), rocket).await;
    let team = team.unwrap();

    // when
    let listed = app
        .list_all_workspaces(admin, search("team", WorkspaceOrder::Name, 0, ""))
        .await;

    // then
    let listed = listed.unwrap().workspaces;
    let found = listed.iter().find(|one| one.id == team.id);
    assert!(team.icon.is_some());
    assert_eq!(found, Some(&team));
}

/// `others_only` keeps the workspaces where the caller is not an owner and has no role.
#[sqlx::test(migrations = false)]
async fn list_all_workspaces_keeps_only_the_others(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, bob, carol) = (app.admin().await, app.user().await, app.user().await);
    name_user(&app, carol, "Carol", "carol").await;
    app.create_workspace(admin, "ops").await.unwrap();
    let shared = app.workspace_of(bob).await;
    app.grant(&[Workspace::MEMBER.of(shared, admin)]).await;
    let mut request = search("", WorkspaceOrder::Name, 0, "");
    request.others_only = true;

    // when
    let others = app.list_all_workspaces(admin, request).await.unwrap();

    // then
    assert_eq!(
        (owners(&others.details), others.total),
        (vec!["Carol".to_owned()], 1)
    );
    assert_eq!(others.details[0].role, "");
}

/// Only an instance admin and a role of the instance with `workspaces.view` open the console.
#[sqlx::test(migrations = false)]
async fn list_all_workspaces_needs_an_instance_role(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, user, viewer) = (app.admin().await, app.user().await, app.user().await);
    let role = RoleInput::on_platform("Workspace auditor", &["workspaces.view"]);
    let role = app.create_role(admin, role).await.unwrap();
    app.grant_on_platform(admin, &role.grant_name, viewer)
        .await
        .unwrap();
    let request = || search("", WorkspaceOrder::Name, 0, "");

    // when
    let refused = app.list_all_workspaces(user, request()).await;
    let allowed = app.list_all_workspaces(viewer, request()).await;

    // then
    assert_code(refused, Code::PermissionDenied);
    assert_eq!(allowed.unwrap().total, 3);
}
