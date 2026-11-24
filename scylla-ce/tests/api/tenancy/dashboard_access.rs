//! The tests of the access to the dashboards: the built-in roles of the workspace, an instance
//! admin, the creator of a dashboard, a private dashboard and the cells of the catalog.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::dashboards::v1::DashboardVisibility;
use scylla_testkit::{text, Role, RoleInput, TestApp};
use sqlx::PgPool;
use tonic::{Code, Status};

use crate::{app, assert_code};

/// The visibility of a dashboard that only its creator sees.
const PRIVATE: DashboardVisibility = DashboardVisibility::Private;
/// The visibility of a dashboard that each reader of the workspace sees.
const SHARED: DashboardVisibility = DashboardVisibility::Workspace;

/// Each caller gets the routes of its rights on a shared dashboard of another actor: an owner, an
/// admin and an instance admin each route, a member the reads and the creation, a viewer the
/// reads, and another user none.
#[sqlx::test(migrations = false)]
async fn each_role_gets_its_dashboard_routes(pool: PgPool) {
    // given
    let app = app(pool).await;
    let author = app.user().await;
    let (owner, admin, member) = (app.user().await, app.user().await, app.user().await);
    let (viewer, stranger, instance_admin) =
        (app.user().await, app.user().await, app.admin().await);
    let workspace = app.workspace_of(author).await.to_string();
    let in_workspace = |name| Role::on("workspace", &workspace, name);
    let grants = [
        (Role::owner_of(&workspace), owner),
        (in_workspace("admin"), admin),
        (in_workspace("member"), member),
        (in_workspace("viewer"), viewer),
    ];
    for (role, user) in grants {
        app.grant_role(author, role, &user.to_string())
            .await
            .unwrap();
    }

    // when
    let mut answers = Vec::new();
    for caller in [owner, admin, member, viewer, stranger, instance_admin] {
        answers.push(routes(&app, author, caller, &workspace).await);
    }

    // then
    let expected = [
        [true; 6],
        [true; 6],
        [true, true, true, false, false, false],
        [true, true, false, false, false, false],
        [false; 6],
        [true; 6],
    ];
    assert_eq!(answers, expected);
}

/// Calls each route as `caller` on a new shared dashboard of `author`, and returns which ones
/// pass: list, get, create, update, set the team default and delete.
async fn routes(app: &TestApp, author: ActorId, caller: ActorId, workspace: &str) -> [bool; 6] {
    let created = app
        .create_dashboard(author, workspace, "Ops", SHARED)
        .await
        .unwrap();
    let dashboard = text(&created.id);
    let default = Some(dashboard);
    [
        passes(app.dashboard_names(caller, workspace).await),
        passes(app.get_dashboard(caller, dashboard).await),
        passes(
            app.create_dashboard(caller, workspace, "Mine", SHARED)
                .await,
        ),
        passes(app.rename_dashboard(caller, dashboard, 1, "Renamed").await),
        passes(
            app.set_team_default_dashboard(caller, workspace, default)
                .await,
        ),
        passes(app.delete_dashboard(caller, dashboard).await),
    ]
}

/// Returns `true` if a call passes, and checks that a refused call is `PERMISSION_DENIED`.
fn passes<T>(result: Result<T, Status>) -> bool {
    match result {
        Ok(_) => true,
        Err(status) => {
            assert_eq!(status.code(), Code::PermissionDenied, "{status:?}");
            false
        }
    }
}

/// A member changes and deletes its own dashboard, but not the dashboard of another member. A
/// creator that leaves the workspace loses its dashboards.
#[sqlx::test(migrations = false)]
async fn the_creator_changes_and_deletes_its_dashboard(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, member, other) = (app.user().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    let as_member = Role::on("workspace", &workspace, "member");
    for user in [member, other] {
        app.grant_role(owner, as_member, &user.to_string())
            .await
            .unwrap();
    }
    let create = |actor| app.create_dashboard(actor, &workspace, "Ops", SHARED);
    let (own, others) = (create(member).await.unwrap(), create(other).await.unwrap());
    let (own, others) = (text(&own.id).to_owned(), text(&others.id).to_owned());
    let kept = create(other).await.unwrap();
    let kept = text(&kept.id).to_owned();

    // when
    let renamed = app.rename_dashboard(member, &own, 1, "Mine").await;
    let changes_other = app.rename_dashboard(member, &others, 1, "Taken").await;
    let deletes_other = app.delete_dashboard(member, &others).await;
    let deleted = app.delete_dashboard(member, &own).await;
    app.revoke_role(owner, as_member, &other.to_string())
        .await
        .unwrap();
    let reads_after_leave = app.get_dashboard(other, &kept).await;
    let changes_after_leave = app.rename_dashboard(other, &kept, 1, "Gone").await;
    let deletes_after_leave = app.delete_dashboard(other, &kept).await;

    // then
    assert_eq!(renamed.unwrap().version, 2);
    assert_code(changes_other, Code::PermissionDenied);
    assert_code(deletes_other, Code::PermissionDenied);
    assert!(deleted.is_ok(), "{deleted:?}");
    assert_code(reads_after_leave, Code::PermissionDenied);
    assert_code(changes_after_leave, Code::PermissionDenied);
    assert_code(deletes_after_leave, Code::PermissionDenied);
}

/// A private dashboard is visible to its creator only: an owner and an instance admin do not
/// see it in the list, and get the answer of an unknown dashboard, with no workspace in it.
#[sqlx::test(migrations = false)]
async fn a_private_dashboard_is_hidden_from_each_other_actor(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, member, instance_admin) = (app.user().await, app.user().await, app.admin().await);
    let workspace = app.workspace_of(owner).await.to_string();
    app.grant_role(
        owner,
        Role::on("workspace", &workspace, "member"),
        &member.to_string(),
    )
    .await
    .unwrap();
    let created = app
        .create_dashboard(member, &workspace, "Mine", PRIVATE)
        .await
        .unwrap();
    let dashboard = text(&created.id);

    for caller in [owner, instance_admin] {
        // when
        let names = app.dashboard_names(caller, &workspace).await.unwrap();
        let read = app.get_dashboard(caller, dashboard).await;
        let changed = app.rename_dashboard(caller, dashboard, 1, "Taken").await;
        let deleted = app.delete_dashboard(caller, dashboard).await;
        let default = Some(dashboard);
        let set = app
            .set_team_default_dashboard(caller, &workspace, default)
            .await;

        // then
        assert!(names.is_empty(), "{names:?}");
        let status = read.expect_err("a hidden dashboard");
        assert_eq!(status.code(), Code::PermissionDenied);
        assert!(status.message().contains(dashboard), "{status:?}");
        assert!(!status.message().contains(&workspace), "{status:?}");
        assert_code(changed, Code::PermissionDenied);
        assert_code(deleted, Code::PermissionDenied);
        assert_code(set, Code::NotFound);
    }
    let names = app.dashboard_names(member, &workspace).await.unwrap();
    assert_eq!(names, ["Mine"]);
    assert!(app.get_dashboard(member, dashboard).await.is_ok());
}

/// A role of the workspace gets the routes of its cells: `dashboards.view` the reads,
/// `dashboards.create` also the creation, and `dashboards.manage` also the change of a shared
/// dashboard of another actor and the team default.
#[sqlx::test(migrations = false)]
async fn a_role_gets_the_routes_of_its_dashboard_cells(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let cells: [&[&str]; 3] = [
        &["workspace.view", "dashboards.view"],
        &["workspace.view", "dashboards.view", "dashboards.create"],
        &["workspace.view", "dashboards.view", "dashboards.manage"],
    ];
    let mut holders = Vec::new();
    for (index, permissions) in cells.into_iter().enumerate() {
        let name = format!("Dashboards {index}");
        let input = RoleInput {
            name: &name,
            level: "workspace",
            holder: "people",
            permissions,
            workspace: Some(&workspace),
        };
        let role = app.create_role(owner, input).await.unwrap();
        let holder = app.user().await;
        let grant = Role::on("workspace", &workspace, &role.grant_name);
        app.grant_role(owner, grant, &holder.to_string())
            .await
            .unwrap();
        holders.push(holder);
    }

    // when
    let mut answers = Vec::new();
    for holder in holders {
        answers.push(routes(&app, owner, holder, &workspace).await);
    }

    // then
    let expected = [
        [true, true, false, false, false, false],
        [true, true, true, false, false, false],
        [true, true, false, true, true, true],
    ];
    assert_eq!(answers, expected);
}
