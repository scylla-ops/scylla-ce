//! The tests of the limits of the inbox: the cap of the count, and the purge.

use scylla_domain::identity::actors::ActorId;
use sqlx::PgPool;

use crate::app;

/// The unread count stops at 100.
#[sqlx::test(migrations = false)]
async fn the_unread_count_stops_at_100(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let user = app.user().await;
    insert_rows(&pool, user, 105, "2 days").await;

    // when
    let summary = app.notification_summary(user).await.unwrap();

    // then
    assert_eq!(summary.unread_count, 100);
}

/// The purge deletes a row older than the retention, a fact older than 30 days, and the oldest read
/// rows above 1,000 for one person.
#[sqlx::test(migrations = false)]
async fn the_purge_deletes_old_rows_and_rows_above_the_limit(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let old = app.user().await;
    let many = app.user().await;
    insert_rows(&pool, old, 3, "100 days").await;
    insert_rows(&pool, many, 1005, "1 day").await;
    sqlx::query!(
        "UPDATE notifications SET read_at = now() WHERE recipient_id = $1",
        many.to_db()
    )
    .execute(&pool)
    .await
    .unwrap();
    app.create_personal_token(many, "last", None).await.unwrap();
    app.dispatch_notifications().await;
    sqlx::query!(
        "INSERT INTO notification_events (id, kind, dispatched_at)
         VALUES (gen_random_uuid(), 'token.created', now() - interval '31 days')"
    )
    .execute(&pool)
    .await
    .unwrap();

    // when
    let retention = std::time::Duration::from_secs(90 * 24 * 60 * 60);
    scylla_core::features::notifications::purge::run(&pool, retention)
        .await
        .unwrap();

    // then
    assert_eq!(count_rows(&pool, old).await, 0);
    assert_eq!(count_rows(&pool, many).await, 1000);
    let inbox = app.inbox(many).await;
    assert_eq!(inbox[0].kind, "token.created");
    let facts = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM notification_events
           WHERE dispatched_at < now() - interval '30 days'"#
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(facts, 0);
}

/// Returns the number of rows of a user.
async fn count_rows(pool: &PgPool, user: ActorId) -> i64 {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM notifications WHERE recipient_id = $1"#,
        user.to_db()
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Inserts rows of a known kind with an age.
async fn insert_rows(pool: &PgPool, user: ActorId, count: i32, age: &str) {
    sqlx::query!(
        "INSERT INTO notifications (id, recipient_id, kind, reason, group_key, first_at, last_at)
         SELECT gen_random_uuid(), $1, 'token.created', 'subject', gen_random_uuid()::text,
                now() - $3::text::interval, now() - $3::text::interval
         FROM generate_series(1, $2)",
        user.to_db(),
        count,
        age,
    )
    .execute(pool)
    .await
    .unwrap();
}
