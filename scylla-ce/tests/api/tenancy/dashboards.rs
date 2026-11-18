//! The tests of `DashboardService`: the creation, the read, the change at a version, the list
//! and the delete of the saved layouts of the dashboard, and the bad values.

use scylla_proto::dashboards::v1::{
    CreateDashboardRequest, DashboardVisibility, DashboardWidgets, UpdateDashboardRequest,
};
use scylla_testkit::{dashboard_widgets, id, text};
use sqlx::PgPool;
use tonic::Code;

use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// The visibility of a dashboard that only its creator sees.
const PRIVATE: DashboardVisibility = DashboardVisibility::Private;
/// The visibility of a dashboard that each reader of the workspace sees.
const SHARED: DashboardVisibility = DashboardVisibility::Workspace;

/// A new dashboard is private when the request sets no visibility, has the version 1, and keeps
/// its widgets as the client wrote them.
#[sqlx::test(migrations = false)]
async fn create_dashboard_keeps_the_widgets(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let json = r#"[{"kind":"success_rate","size":"m","x":0,"y":0}]"#;
    let request = CreateDashboardRequest {
        workspace_id: Some(id(&workspace)),
        name: "  Ops overview  ".to_owned(),
        visibility: DashboardVisibility::Unspecified.into(),
        widgets: Some(DashboardWidgets {
            schema_version: 3,
            widgets_json: json.to_owned(),
        }),
    };

    // when
    let created = app.create_dashboard_with(owner, request).await.unwrap();
    let read = app.get_dashboard(owner, text(&created.id)).await.unwrap();

    // then
    assert_eq!(created.name, "Ops overview");
    assert_eq!(created.visibility, PRIVATE as i32);
    assert_eq!((created.version, created.team_default), (1, false));
    assert_eq!(text(&created.created_by), owner.to_string());
    assert_eq!(text(&created.workspace_id), workspace);
    assert_eq!(read.dashboard.unwrap(), created);
    let widgets = read.widgets.unwrap();
    assert_eq!(widgets.schema_version, 3);
    let value: serde_json::Value = serde_json::from_str(&widgets.widgets_json).unwrap();
    assert_eq!(
        value,
        serde_json::from_str::<serde_json::Value>(json).unwrap()
    );
}

/// A bad name, a bad visibility, missing widgets, a version of the format out of its range, a
/// text that is not JSON and a text above 64 KiB are request errors.
#[sqlx::test(migrations = false)]
async fn create_dashboard_refuses_bad_values(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let base = CreateDashboardRequest {
        workspace_id: Some(id(&workspace)),
        name: "Ops".to_owned(),
        visibility: SHARED.into(),
        widgets: Some(dashboard_widgets("{}")),
    };
    let with = |change: &dyn Fn(&mut CreateDashboardRequest)| {
        let mut request = base.clone();
        change(&mut request);
        request
    };
    let large = format!("[\"{}\"]", "a".repeat(64 << 10));
    let cases = [
        ("name", with(&|request| request.name = " ".to_owned())),
        ("visibility", with(&|request| request.visibility = 9)),
        ("widgets", with(&|request| request.widgets = None)),
        (
            "widgets.schema_version",
            with(&|request| request.widgets = Some(versioned(0))),
        ),
        (
            "widgets.schema_version",
            with(&|request| request.widgets = Some(versioned(65_536))),
        ),
        (
            "widgets.widgets_json",
            with(&|request| request.widgets = Some(dashboard_widgets("{not json"))),
        ),
        (
            "widgets.widgets_json",
            with(&|request| request.widgets = Some(dashboard_widgets(&large))),
        ),
    ];

    for (field, request) in cases {
        // when
        let result = app.create_dashboard_with(owner, request).await;

        // then
        assert_invalid_field(&result.expect_err("a bad value"), field);
    }
}

/// Returns widgets with a version of the format.
fn versioned(schema_version: u32) -> DashboardWidgets {
    DashboardWidgets {
        schema_version,
        widgets_json: "[]".to_owned(),
    }
}

/// A change needs the version that the client read: it gives the next version, and a stale
/// version gives `ABORTED`. A field that the request does not set does not change.
#[sqlx::test(migrations = false)]
async fn update_dashboard_needs_the_version_read(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let created = app
        .create_dashboard(owner, &workspace, "Ops", PRIVATE)
        .await
        .unwrap();
    let dashboard = text(&created.id).to_owned();
    let change = UpdateDashboardRequest {
        dashboard_id: Some(id(&dashboard)),
        version: 1,
        name: None,
        visibility: SHARED.into(),
        widgets: Some(dashboard_widgets(r#"{"widgets":[]}"#)),
    };

    // when
    let changed = app.update_dashboard(owner, change.clone()).await.unwrap();
    let stale = app.update_dashboard(owner, change).await;
    let renamed = app
        .rename_dashboard(owner, &dashboard, 2, "Release day")
        .await
        .unwrap();
    let read = app.get_dashboard(owner, &dashboard).await.unwrap();

    // then
    assert_eq!((changed.version, changed.visibility), (2, SHARED as i32));
    assert_eq!(changed.name, "Ops");
    assert_code(stale, Code::Aborted);
    assert_eq!((renamed.version, renamed.visibility), (3, SHARED as i32));
    assert_eq!(read.widgets.unwrap().widgets_json, r#"{"widgets":[]}"#);
}

/// The list gives the shared dashboards and the private dashboards of the caller, oldest first,
/// in pages.
#[sqlx::test(migrations = false)]
async fn list_dashboards_gives_the_shared_and_the_own_dashboards(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    for (name, visibility) in [("one", SHARED), ("two", PRIVATE), ("three", SHARED)] {
        app.create_dashboard(owner, &workspace, name, visibility)
            .await
            .unwrap();
    }

    // when
    let first = app
        .list_dashboards_page(owner, &workspace, 2, "")
        .await
        .unwrap();
    let token = first.page.clone().unwrap().next_token;
    let second = app
        .list_dashboards_page(owner, &workspace, 2, &token)
        .await
        .unwrap();

    // then
    let names = |page: &[scylla_proto::dashboards::v1::Dashboard]| {
        page.iter().map(|one| one.name.clone()).collect::<Vec<_>>()
    };
    assert_eq!(names(&first.dashboards), ["one", "two"]);
    assert_eq!(names(&second.dashboards), ["three"]);
    assert!(second.page.unwrap().next_token.is_empty());
}

/// A deleted dashboard and an unknown dashboard get the answer of a refusal.
#[sqlx::test(migrations = false)]
async fn delete_dashboard_removes_it(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let created = app
        .create_dashboard(owner, &workspace, "Ops", SHARED)
        .await
        .unwrap();
    let dashboard = text(&created.id).to_owned();

    // when
    app.delete_dashboard(owner, &dashboard).await.unwrap();
    let read = app.get_dashboard(owner, &dashboard).await;
    let deleted_again = app.delete_dashboard(owner, &dashboard).await;
    let unknown = app.rename_dashboard(owner, UNKNOWN_ID, 1, "Other").await;
    let names = app.dashboard_names(owner, &workspace).await.unwrap();

    // then
    assert_code(read, Code::PermissionDenied);
    assert_code(deleted_again, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
    assert!(names.is_empty());
}
