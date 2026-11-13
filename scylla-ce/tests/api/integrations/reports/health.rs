//! The tests of the health of the GitLab tokens: a warning 14 days before the expiry, and a
//! connection that needs a person after it.

use std::time::Duration;

use scylla_domain::identity::actors::ActorId;
use scylla_proto::integrations::v1::ConnectionState;
use scylla_proto::notifications::v1::Notification;
use scylla_testkit::{FakeProvider, TestApp};
use sqlx::PgPool;

use super::{app_with, linked};

/// Waits until the inbox of a person has a row of a kind, and returns the inbox. Fails after 15
/// seconds.
async fn wait_for_notice(app: &TestApp, actor: ActorId, kind: &str) -> Vec<Notification> {
    let start = tokio::time::Instant::now();
    loop {
        let inbox = app.inbox(actor).await;
        if inbox.iter().any(|row| row.kind == kind) {
            return inbox;
        }
        assert!(start.elapsed() < Duration::from_secs(15), "no {kind}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// A GitLab token that expires within 14 days warns the owners one time, with the day of the
/// expiry. The connection stays active.
#[sqlx::test(migrations = false)]
async fn a_token_that_expires_soon_warns_the_owners(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "gitlab").await;

    // when
    app.set_token_expiry(&linked.connection, 5).await;
    wait_for_notice(&app, linked.owner, "vcs.token_expiring").await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let inbox = app.inbox(linked.owner).await;
    let connection = app
        .get_vcs_connection(linked.owner, &linked.connection)
        .await
        .unwrap();

    // then
    let rows: Vec<_> = inbox
        .iter()
        .filter(|row| row.kind == "vcs.token_expiring")
        .collect();
    assert_eq!(rows.len(), 1);
    let day: String = sqlx::query_scalar(
        "SELECT to_char(token_expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD')
         FROM vcs_connections WHERE id = $1::uuid",
    )
    .bind(&linked.connection)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(rows[0].params["expires_at"], day);
    assert_eq!(rows[0].params["account"], "scylla-bot");
    assert_eq!(rows[0].category, "integrations");
    assert_eq!(connection.state(), ConnectionState::Active);
}

/// A GitLab token that expired moves its connection to `needs_attention` with `TOKEN_EXPIRED`,
/// and the owners get a notice.
#[sqlx::test(migrations = false)]
async fn an_expired_token_needs_attention(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "gitlab").await;

    // when
    app.set_token_expiry(&linked.connection, -1).await;
    let inbox = wait_for_notice(&app, linked.owner, "vcs.connection_broken").await;
    let connection = app
        .get_vcs_connection(linked.owner, &linked.connection)
        .await
        .unwrap();

    // then
    let row = inbox
        .iter()
        .find(|row| row.kind == "vcs.connection_broken")
        .unwrap();
    assert_eq!(row.params["reason"], "TOKEN_EXPIRED");
    assert_eq!(connection.state(), ConnectionState::NeedsAttention);
    assert_eq!(connection.state_reason, "TOKEN_EXPIRED");
}
