//! The tests of the delete of the GitHub App of an instance: its connections stop, and a new App
//! can be made.

use scylla_proto::integrations::v1::ConnectionState;
use scylla_testkit::text;
use sqlx::PgPool;
use tonic::Code;

use super::{app_with_fake, created};
use crate::assert_code;

/// A delete removes the App, and each of its connections becomes disconnected with the reason
/// `APP_UNINSTALLED`: the owners of the workspace get a notice. A second delete gives
/// `NOT_FOUND`, and a new start works.
#[sqlx::test(migrations = false)]
async fn delete_vcs_app_disconnects_its_connections(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool, false).await;
    let (admin, made) = created(&app, &fake).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let connection = app
        .insert_github_connection(workspace, owner, 42, "acme")
        .await;
    app.set_connection_app(&connection, text(&made.id)).await;

    // when
    app.delete_vcs_app(admin).await.unwrap();
    let read = app.get_vcs_app(admin).await.unwrap();
    let stopped = app.get_vcs_connection(owner, &connection).await.unwrap();
    let inbox = app.inbox(owner).await;
    let again = app.delete_vcs_app(admin).await;
    let restarted = app.start_github_manifest(admin, "").await;

    // then
    assert!(read.is_none());
    assert_eq!(stopped.state(), ConnectionState::Disconnected);
    assert_eq!(stopped.state_reason, "APP_UNINSTALLED");
    let notice = inbox
        .iter()
        .find(|row| row.kind == "vcs.connection_broken")
        .expect("a notice of the connection");
    assert_eq!(notice.params["reason"], "APP_UNINSTALLED");
    assert_code(again, Code::NotFound);
    assert!(restarted.is_ok());
}
