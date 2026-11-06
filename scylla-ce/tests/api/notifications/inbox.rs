//! The tests of the routes of the inbox: the counts, read, done, the filters and the pages.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::common::v1::PageRequest;
use scylla_proto::notifications::v1::ListNotificationsRequest;
use scylla_testkit::TestApp;
use sqlx::PgPool;

use super::row_id;
use crate::app;

/// Gives `count` rows that need no action to a user: one fact for each personal token.
async fn rows_for(app: &TestApp, user: ActorId, count: usize) {
    for index in 0..count {
        app.create_personal_token(user, &format!("token-{index}"), None)
            .await
            .unwrap();
    }
    app.dispatch_notifications().await;
}

/// Read changes the count of the unread rows, and unread changes it back.
#[sqlx::test(migrations = false)]
async fn read_and_unread_change_the_count(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    rows_for(&app, user, 2).await;
    let inbox = app.inbox(user).await;
    let first = row_id(&inbox[0]);

    // when
    let before = app.notification_summary(user).await.unwrap();
    app.mark_notifications_read(user, &[first], true)
        .await
        .unwrap();
    let read = app.notification_summary(user).await.unwrap();
    app.mark_notifications_read(user, &[first], false)
        .await
        .unwrap();
    let unread = app.notification_summary(user).await.unwrap();

    // then
    assert_eq!(before.unread_count, 2);
    assert_eq!(before.poll_after_seconds, 30);
    assert_eq!(read.unread_count, 1);
    assert_eq!(unread.unread_count, 2);
}

/// Mark all as read skips a row that waits for an action, and marking this row as read has no
/// effect.
#[sqlx::test(migrations = false)]
async fn mark_all_skips_a_row_that_needs_an_action(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let email = "wait@example.com";
    let user = app.user_with(email, None).await;
    app.create_invitation(owner, workspace, email, "viewer")
        .await
        .unwrap();
    rows_for(&app, user, 1).await;

    // when
    app.mark_all_notifications_read(user).await.unwrap();
    let summary = app.notification_summary(user).await.unwrap();
    let inbox = app.inbox(user).await;
    app.mark_notifications_read(user, &[row_id(&inbox[0])], true)
        .await
        .unwrap();
    let after = app.notification_summary(user).await.unwrap();

    // then
    assert_eq!(summary.unread_count, 1);
    assert_eq!(summary.needs_action_count, 1);
    assert_eq!(inbox[0].kind, "invitation.received");
    assert!(inbox[0].read_at.is_none());
    assert!(inbox[1].read_at.is_some());
    assert_eq!(after.unread_count, 1);
}

/// A done row leaves the inbox and the count, and it is in the list of the done rows.
#[sqlx::test(migrations = false)]
async fn done_moves_a_row_out_of_the_inbox(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    rows_for(&app, user, 2).await;
    let inbox = app.inbox(user).await;
    let id = row_id(&inbox[0]);
    let done = || ListNotificationsRequest {
        done: true,
        ..Default::default()
    };

    // when
    app.set_notifications_done(user, &[id], true).await.unwrap();
    let left = app.inbox(user).await;
    let summary = app.notification_summary(user).await.unwrap();
    let list = app.list_notifications(user, done()).await.unwrap();
    app.set_notifications_done(user, &[id], false)
        .await
        .unwrap();
    let back = app.inbox(user).await;

    // then
    assert_eq!(left.len(), 1);
    assert_eq!(summary.unread_count, 1);
    assert_eq!(list.notifications.len(), 1);
    assert_eq!(row_id(&list.notifications[0]), id);
    assert_eq!(back.len(), 2);
}

/// The category, the workspace and the unread filters reduce the list.
#[sqlx::test(migrations = false)]
async fn the_filters_reduce_the_list(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let email = "filter@example.com";
    let user = app.user_with(email, None).await;
    app.create_invitation(owner, workspace, email, "viewer")
        .await
        .unwrap();
    rows_for(&app, user, 2).await;
    let inbox = app.inbox(user).await;
    let token_row = inbox
        .iter()
        .find(|row| row.kind == "token.created")
        .unwrap();
    app.mark_notifications_read(user, &[row_id(token_row)], true)
        .await
        .unwrap();
    let count = |request: ListNotificationsRequest| {
        let app = &app;
        async move {
            let page = app.list_notifications(user, request).await.unwrap();
            page.notifications.len()
        }
    };

    // when
    let security = count(ListNotificationsRequest {
        category: "security".to_owned(),
        ..Default::default()
    })
    .await;
    let unknown = count(ListNotificationsRequest {
        category: "nothing".to_owned(),
        ..Default::default()
    })
    .await;
    let unread = count(ListNotificationsRequest {
        unread_only: true,
        ..Default::default()
    })
    .await;
    let action = count(ListNotificationsRequest {
        needs_action_only: true,
        ..Default::default()
    })
    .await;
    let in_workspace = count(ListNotificationsRequest {
        workspace_id: Some(scylla_testkit::id(workspace.to_string())),
        ..Default::default()
    })
    .await;

    // then
    assert_eq!(security, 2);
    assert_eq!(unknown, 0);
    assert_eq!(unread, 2);
    assert_eq!(action, 1);
    assert_eq!(in_workspace, 1);
}

/// The pages give each row one time, the newest first.
#[sqlx::test(migrations = false)]
async fn the_pages_give_each_row_one_time(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    rows_for(&app, user, 5).await;
    let page = |token: String| ListNotificationsRequest {
        page: Some(PageRequest { size: 2, token }),
        ..Default::default()
    };

    // when
    let mut seen = Vec::new();
    let mut token = String::new();
    loop {
        let answer = app.list_notifications(user, page(token)).await.unwrap();
        seen.extend(
            answer
                .notifications
                .iter()
                .map(|row| row_id(row).to_owned()),
        );
        token = answer.page.unwrap().next_token;
        if token.is_empty() {
            break;
        }
    }

    // then
    let mut sorted = seen.clone();
    sorted.sort();
    sorted.reverse();
    assert_eq!(seen.len(), 5);
    assert_eq!(seen, sorted);
}
