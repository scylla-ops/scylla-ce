//! The tests of the role App creator of a workspace: it creates Apps, it manages the Apps that it
//! created only, and the web app can ask for the permission before it shows the button.

use scylla_domain::identity::apps::AppHolder;
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_testkit::text;
use sqlx::PgPool;
use tonic::Code;

use crate::{app, assert_code};

/// An App creator creates an App and manages it, and gets no right on the other Apps of the
/// workspace.
#[sqlx::test(migrations = false)]
async fn an_app_creator_manages_its_own_apps(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, creator) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[AppHolder::APP_CREATOR.of(workspace, creator)])
        .await;
    let (other, _) = app.app_in(owner, workspace, false).await;

    // when
    let created = app.create_app(creator, workspace, false).await.unwrap();
    let mine = text(&created.app.unwrap().id).to_owned();
    let read = app.get_app(creator, &mine).await;
    let token = app.create_app_token(creator, &mine, "ci", 0).await;
    let code = app.create_join_code(creator, &mine).await;
    let others = app.create_app_token(creator, &other.app, "ci", 0).await;
    let listed = app.list_apps(creator, workspace, None, 10, "").await;
    let deleted = app.delete_app(creator, &mine).await;

    // then
    assert!(read.is_ok() && token.is_ok() && code.is_ok());
    assert_code(others, Code::PermissionDenied);
    assert_code(listed, Code::PermissionDenied);
    assert!(deleted.is_ok());
}

/// The admins and the App creators have `create_app`, and a member does not.
#[sqlx::test(migrations = false)]
async fn check_permissions_tells_who_creates_apps(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, creator, member) = (app.user().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[
        AppHolder::APP_CREATOR.of(workspace, creator),
        Workspace::MEMBER.of(workspace, member),
    ])
    .await;
    let id = workspace.to_string();

    let ask = |actor| app.check_permissions(actor, "workspace", &id, &["create_app"]);

    // when
    let by_owner = ask(owner).await.unwrap();
    let by_creator = ask(creator).await.unwrap();
    let by_member = ask(member).await.unwrap();
    let created = app.create_app(member, workspace, false).await;

    // then
    assert_eq!(
        (by_owner, by_creator, by_member),
        (vec![true], vec![true], vec![false])
    );
    assert_code(created, Code::PermissionDenied);
}
