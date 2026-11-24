//! The tests of a new install of the GitHub App on an account whose connection stopped: the new
//! installation takes over the connection, and its links work again.

use scylla_proto::integrations::v1::ConnectionState;
use scylla_testkit::{text, LinkRow};
use sqlx::PgPool;

use super::{app_with_fake, permissions};

/// A new install on the account of a connection that the uninstall of the App disconnected takes
/// over this connection: same id, the new installation, the state of its permissions, and the
/// link of the connection is active again. No second connection is made.
#[sqlx::test(migrations = false)]
async fn complete_github_install_takes_over_the_stopped_connection_of_the_account(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace_id = app.workspace_of(owner).await;
    let workspace = workspace_id.to_string();
    let old = app
        .insert_github_connection(workspace_id, owner, 42, "acme")
        .await;
    let project = app.create_project(owner, "API").await.unwrap();
    let project = text(&project.id).to_owned();
    let link = LinkRow {
        project: &project,
        connection: &old,
        external_id: "7",
        full_name: "acme/api",
        default_branch: "main",
    };
    app.insert_repository_link(&link).await;
    app.set_connection_state(&old, "disconnected", "APP_UNINSTALLED")
        .await;
    fake.answer_github_install(43, "acme", permissions(), &[(7, "acme/api", false)]);

    // when
    let taken = app.install_github(owner, &workspace, 43).await.unwrap();
    let link = app.get_repository_link(owner, &project).await.unwrap();
    let listed = app.list_vcs_connections(owner, &workspace, 0, "").await;

    // then
    assert_eq!(text(&taken.id), old);
    assert_eq!(taken.external_id, "43");
    assert_eq!(taken.state(), ConnectionState::Active);
    assert!(taken.state_reason.is_empty());
    assert_eq!(taken.missing_permissions, ["administration"]);
    assert_eq!((taken.repository_count, taken.all_repositories), (1, false));
    let link = link.expect("a link");
    assert_eq!(text(&link.connection_id), old);
    assert_eq!(link.state(), ConnectionState::Active);
    assert_eq!(listed.unwrap().connections.len(), 1);
}

/// When two stopped connections of the account match, the new installation takes over the
/// newest one, and the other one stays as it is.
#[sqlx::test(migrations = false)]
async fn complete_github_install_takes_over_the_newest_stopped_connection(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace_id = app.workspace_of(owner).await;
    let workspace = workspace_id.to_string();
    let older = app
        .insert_github_connection(workspace_id, owner, 41, "acme")
        .await;
    let newer = app
        .insert_github_connection(workspace_id, owner, 42, "acme")
        .await;
    app.set_connection_state(&older, "disconnected", "APP_UNINSTALLED")
        .await;
    app.set_connection_state(&newer, "suspended", "APP_SUSPENDED")
        .await;
    fake.answer_github_install(43, "acme", permissions(), &[]);

    // when
    let taken = app.install_github(owner, &workspace, 43).await.unwrap();
    let kept = app.get_vcs_connection(owner, &older).await.unwrap();

    // then
    assert_eq!(text(&taken.id), newer);
    assert_eq!(taken.external_id, "43");
    assert_eq!(taken.state(), ConnectionState::Active);
    assert_eq!(kept.external_id, "41");
    assert_eq!(kept.state(), ConnectionState::Disconnected);
}

/// A new install on another account makes a new connection, and so does a new install on an
/// account whose connection is active. The old connections do not change.
#[sqlx::test(migrations = false)]
async fn complete_github_install_makes_a_new_connection_for_another_account(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace_id = app.workspace_of(owner).await;
    let workspace = workspace_id.to_string();
    let stopped = app
        .insert_github_connection(workspace_id, owner, 42, "acme")
        .await;
    let active = app
        .insert_github_connection(workspace_id, owner, 50, "octo")
        .await;
    app.set_connection_state(&stopped, "disconnected", "APP_UNINSTALLED")
        .await;
    fake.answer_github_install(43, "globex", permissions(), &[]);
    fake.answer_github_install(51, "octo", permissions(), &[]);

    // when
    let other = app.install_github(owner, &workspace, 43).await.unwrap();
    let second = app.install_github(owner, &workspace, 51).await.unwrap();
    let kept = app.get_vcs_connection(owner, &stopped).await.unwrap();
    let listed = app.list_vcs_connections(owner, &workspace, 0, "").await;

    // then
    assert_ne!(text(&other.id), stopped);
    assert_eq!(
        (other.account.as_str(), other.external_id.as_str()),
        ("globex", "43")
    );
    assert_ne!(text(&second.id), active);
    assert_eq!(second.external_id, "51");
    assert_eq!(kept.external_id, "42");
    assert_eq!(kept.state(), ConnectionState::Disconnected);
    assert_eq!(listed.unwrap().connections.len(), 4);
}
