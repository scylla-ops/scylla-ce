//! The tests of the disable and the enable of an App: its tokens stop at once, its key signs no
//! new token until an admin enables it, and only the admins of the workspace change it.

use scylla_domain::tenancy::workspaces::Workspace;
use scylla_proto::projects::v1::ListProjectsRequest;
use scylla_testkit::{with_token, TestApp};
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

use crate::{app, assert_code};

/// Returns `Ok` if a token can call a route that each actor can call.
async fn try_token(app: &TestApp, token: &str) -> Result<(), Status> {
    let list = ListProjectsRequest {
        workspace_id: None,
        page: None,
    };
    app.projects
        .clone()
        .list_projects(with_token(token, list))
        .await
        .map(|_| ())
}

/// A disabled App loses its token at once, and its key gets `APP_DISABLED`. A second disable
/// keeps the first time. Enabled again, its key signs in again.
#[sqlx::test(migrations = false)]
async fn disable_app_stops_its_tokens_until_enable_app(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (key, token) = app.app_in(owner, workspace, true).await;

    // when
    let disabled = app.disable_app(owner, &key.app).await.unwrap();
    let again = app.disable_app(owner, &key.app).await.unwrap();
    let old_token = try_token(&app, &token).await;
    let exchange = app.exchange(key.assertion(60)).await;
    let read = app.get_app(owner, &key.app).await.unwrap();
    let enabled = app.enable_app(owner, &key.app).await.unwrap();
    let new_token = app.exchange(key.assertion(60)).await.unwrap();

    // then
    assert!(disabled.disabled_at.is_some());
    assert_eq!(again.disabled_at, disabled.disabled_at);
    assert_code(old_token, Code::Unauthenticated);
    let refused = exchange.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    let reason = refused.get_details_error_info().map(|info| info.reason);
    assert_eq!(reason.as_deref(), Some("APP_DISABLED"));
    assert_eq!(read.app.unwrap().disabled_at, disabled.disabled_at);
    assert!(enabled.disabled_at.is_none());
    assert!(try_token(&app, &new_token).await.is_ok());
}

/// Only the admins of the workspace disable and enable an App, and an unknown App gets the
/// answer of a refusal.
#[sqlx::test(migrations = false)]
async fn disable_and_enable_need_an_admin(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, member) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[Workspace::MEMBER.of(workspace, member)]).await;
    let (key, _) = app.app_in(owner, workspace, true).await;

    // when
    let disabled = app.disable_app(member, &key.app).await;
    let enabled = app.enable_app(member, &key.app).await;
    let unknown = app.disable_app(owner, crate::UNKNOWN_ID).await;

    // then
    assert_code(disabled, Code::PermissionDenied);
    assert_code(enabled, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
}
