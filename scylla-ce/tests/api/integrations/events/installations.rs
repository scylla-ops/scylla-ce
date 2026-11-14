//! The tests of the changes of an installation of the GitHub App: an uninstall, a suspend and
//! its end, new permissions, and a repository that leaves the installation.

use scylla_proto::integrations::v1::ConnectionState;
use serde_json::{json, Value};
use sqlx::PgPool;

use super::{app_with_fake, linked, post_app};

/// Returns an `installation` delivery of the installation 42 with an action.
fn installation(action: &str, permissions: Value) -> Value {
    json!({
        "action": action,
        "installation": { "id": 42, "permissions": permissions },
        "sender": { "login": "octo-owner" },
    })
}

/// Returns an `installation_repositories` delivery of the installation 42.
fn repositories(added: &[u64], removed: &[u64]) -> Value {
    let list = |ids: &[u64]| -> Vec<Value> {
        ids.iter()
            .map(|id| json!({ "id": id, "full_name": format!("octo/{id}") }))
            .collect()
    };
    json!({
        "action": if added.is_empty() { "removed" } else { "added" },
        "installation": { "id": 42 },
        "repository_selection": "selected",
        "repositories_added": list(added),
        "repositories_removed": list(removed),
    })
}

/// An uninstall disconnects the connection with `APP_UNINSTALLED`, the link shows it, and the
/// owner of the workspace gets a notice.
#[sqlx::test(migrations = false)]
async fn an_uninstall_disconnects_the_connection(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;

    // when
    let deleted = post_app(
        &app,
        "installation",
        "d-1",
        &installation("deleted", json!({})),
    )
    .await;
    app.wait_for_delivery("d-1").await;
    let connection = app
        .get_vcs_connection(linked.owner, &linked.connection)
        .await
        .unwrap();
    let link = app
        .get_repository_link(linked.owner, &linked.project)
        .await
        .unwrap()
        .unwrap();
    let inbox = app.inbox(linked.owner).await;

    // then
    assert_eq!(deleted.0, 202);
    assert_eq!(connection.state(), ConnectionState::Disconnected);
    assert_eq!(connection.state_reason, "APP_UNINSTALLED");
    assert_eq!(link.state(), ConnectionState::Disconnected);
    assert_eq!(link.state_reason, "APP_UNINSTALLED");
    let notice = inbox
        .iter()
        .find(|row| row.kind == "vcs.connection_broken")
        .expect("a notice for the owner");
    assert_eq!(notice.params["reason"], "APP_UNINSTALLED");
}

/// A suspend stops the connection with `APP_SUSPENDED`, and its end makes it active again.
#[sqlx::test(migrations = false)]
async fn a_suspend_stops_the_connection_until_its_end(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;

    // when
    post_app(
        &app,
        "installation",
        "d-1",
        &installation("suspend", json!({})),
    )
    .await;
    app.wait_for_delivery("d-1").await;
    let suspended = app
        .get_vcs_connection(linked.owner, &linked.connection)
        .await
        .unwrap();
    post_app(
        &app,
        "installation",
        "d-2",
        &installation("unsuspend", json!({})),
    )
    .await;
    app.wait_for_delivery("d-2").await;
    let active = app
        .get_vcs_connection(linked.owner, &linked.connection)
        .await
        .unwrap();

    // then
    assert_eq!(suspended.state(), ConnectionState::Suspended);
    assert_eq!(suspended.state_reason, "APP_SUSPENDED");
    assert_eq!(active.state(), ConnectionState::Active);
    assert!(active.state_reason.is_empty());
}

/// New permissions with a missing permission that Scylla needs give `PERMISSIONS_NOT_ACCEPTED`,
/// and all the permissions make the connection active again.
#[sqlx::test(migrations = false)]
async fn new_permissions_change_the_state_of_the_connection(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    let missing = json!({ "metadata": "read", "contents": "read" });
    let all = json!({
        "checks": "write", "contents": "read", "pull_requests": "read", "metadata": "read",
    });

    // when
    let action = "new_permissions_accepted";
    post_app(&app, "installation", "d-1", &installation(action, missing)).await;
    app.wait_for_delivery("d-1").await;
    let blocked = app
        .get_vcs_connection(linked.owner, &linked.connection)
        .await
        .unwrap();
    post_app(&app, "installation", "d-2", &installation(action, all)).await;
    app.wait_for_delivery("d-2").await;
    let active = app
        .get_vcs_connection(linked.owner, &linked.connection)
        .await
        .unwrap();

    // then
    assert_eq!(blocked.state(), ConnectionState::NeedsAttention);
    assert_eq!(blocked.state_reason, "PERMISSIONS_NOT_ACCEPTED");
    assert_eq!(active.state(), ConnectionState::Active);
    assert_eq!(active.missing_permissions, vec!["administration"]);
}

/// An App that still asks for `statuses: write` keeps its connection active, with no missing
/// permission and no reason.
#[sqlx::test(migrations = false)]
async fn an_old_app_with_statuses_stays_active(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    let old_app = json!({
        "checks": "write", "statuses": "write", "contents": "read",
        "pull_requests": "read", "metadata": "read", "administration": "write",
    });

    // when
    let action = "new_permissions_accepted";
    post_app(&app, "installation", "d-1", &installation(action, old_app)).await;
    app.wait_for_delivery("d-1").await;
    let connection = app
        .get_vcs_connection(linked.owner, &linked.connection)
        .await
        .unwrap();

    // then
    assert_eq!(connection.state(), ConnectionState::Active);
    assert!(connection.state_reason.is_empty());
    assert!(connection.missing_permissions.is_empty());
}

/// A repository that leaves the installation stops its link with `REPOSITORY_NOT_VISIBLE`, and
/// the link is active again when the repository comes back.
#[sqlx::test(migrations = false)]
async fn a_removed_repository_stops_its_link(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;

    // when
    let event = "installation_repositories";
    post_app(&app, event, "d-1", &repositories(&[], &[7])).await;
    app.wait_for_delivery("d-1").await;
    let removed = app.link_mode(&linked.project).await;
    post_app(&app, event, "d-2", &repositories(&[7, 8], &[])).await;
    app.wait_for_delivery("d-2").await;
    let added = app.link_mode(&linked.project).await;

    // then
    assert_eq!(
        (removed.state.as_str(), removed.reason.as_str()),
        ("needs_attention", "REPOSITORY_NOT_VISIBLE")
    );
    assert_eq!(
        (added.state.as_str(), added.reason.as_str()),
        ("active", "")
    );
    let (count, all) = app.connection_repositories(&linked.connection).await;
    assert_eq!((count, all), (2, false));
}
