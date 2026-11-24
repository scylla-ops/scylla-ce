//! The helpers of the tests of the notifications.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::notifications::v1::Notification;
use scylla_testkit::{text, TestApp};

/// Returns the keys of the kinds in the inbox of a user, after one pass of the task.
pub(super) async fn kinds_of(app: &TestApp, user: ActorId) -> Vec<String> {
    let inbox = app.inbox(user).await;
    let mut kinds: Vec<String> = inbox.into_iter().map(|row| row.kind).collect();
    kinds.sort();
    kinds
}

/// Returns the row of a kind in the inbox of a user.
pub(super) async fn row_of(app: &TestApp, user: ActorId, kind: &str) -> Notification {
    app.inbox(user)
        .await
        .into_iter()
        .find(|row| row.kind == kind)
        .unwrap_or_else(|| panic!("no row {kind}"))
}

/// Returns the text of the id of a row.
pub(super) fn row_id(row: &Notification) -> &str {
    text(&row.id)
}
