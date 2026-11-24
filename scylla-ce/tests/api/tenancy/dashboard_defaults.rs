//! The tests of the team default dashboard of a workspace.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::dashboards::v1::{DashboardVisibility, UpdateDashboardRequest};
use scylla_testkit::{id, text, TestApp};
use sqlx::PgPool;
use tonic::Code;

use crate::{app, assert_code, UNKNOWN_ID};

/// The visibility of a dashboard that only its creator sees.
const PRIVATE: DashboardVisibility = DashboardVisibility::Private;
/// The visibility of a dashboard that each reader of the workspace sees.
const SHARED: DashboardVisibility = DashboardVisibility::Workspace;

/// A workspace has at most one team default: a new one replaces the old one, a private or an
/// unknown dashboard gives `NOT_FOUND`, a dashboard that becomes private stops being the default,
/// and a request with no dashboard removes the default.
#[sqlx::test(migrations = false)]
async fn set_team_default_dashboard_keeps_one_default(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, other) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    let abroad = app.workspace_of(other).await.to_string();
    let create = |name: &'static str, visibility| {
        let (app, workspace) = (&app, &workspace);
        async move {
            let created = app
                .create_dashboard(owner, workspace, name, visibility)
                .await
                .unwrap();
            text(&created.id).to_owned()
        }
    };
    let (one, two, private) = (
        create("one", SHARED).await,
        create("two", SHARED).await,
        create("mine", PRIVATE).await,
    );
    let foreign = app
        .create_dashboard(other, &abroad, "foreign", SHARED)
        .await
        .unwrap();

    // when
    let first = app
        .set_team_default_dashboard(owner, &workspace, Some(&one))
        .await;
    let second = app
        .set_team_default_dashboard(owner, &workspace, Some(&two))
        .await;
    let defaults = default_names(&app, owner, &workspace).await;
    let of_private = app
        .set_team_default_dashboard(owner, &workspace, Some(&private))
        .await;
    let of_abroad = app
        .set_team_default_dashboard(owner, &workspace, Some(text(&foreign.id)))
        .await;
    let unknown = app
        .set_team_default_dashboard(owner, &workspace, Some(UNKNOWN_ID))
        .await;
    let hidden = app
        .update_dashboard(owner, private_request(&two, 1))
        .await
        .unwrap();
    let after_private = default_names(&app, owner, &workspace).await;
    app.set_team_default_dashboard(owner, &workspace, Some(&one))
        .await
        .unwrap();
    let removed = app
        .set_team_default_dashboard(owner, &workspace, None)
        .await;
    let after_removal = default_names(&app, owner, &workspace).await;

    // then
    assert!(first.unwrap().unwrap().team_default);
    assert_eq!(second.unwrap().unwrap().name, "two");
    assert_eq!(defaults, ["two"]);
    assert_code(of_private, Code::NotFound);
    assert_code(of_abroad, Code::NotFound);
    assert_code(unknown, Code::NotFound);
    assert!(!hidden.team_default);
    assert!(after_private.is_empty());
    assert_eq!(removed.unwrap(), None);
    assert!(after_removal.is_empty());
}

/// Returns the request that makes a dashboard private at a version.
fn private_request(dashboard: &str, version: i64) -> UpdateDashboardRequest {
    UpdateDashboardRequest {
        dashboard_id: Some(id(dashboard)),
        version,
        name: None,
        visibility: PRIVATE.into(),
        widgets: None,
    }
}

/// Returns the names of the team default dashboards in the list of an actor.
async fn default_names(app: &TestApp, actor: ActorId, workspace: &str) -> Vec<String> {
    let page = app
        .list_dashboards_page(actor, workspace, 100, "")
        .await
        .unwrap();
    page.dashboards
        .into_iter()
        .filter(|dashboard| dashboard.team_default)
        .map(|dashboard| dashboard.name)
        .collect()
}
