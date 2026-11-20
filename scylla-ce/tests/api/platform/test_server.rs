//! The tests of the pool of the test server on the test database.

use std::time::Duration;

use sqlx::PgPool;
use tokio::time::timeout;

use crate::app;

/// The test server and its tasks use a pool of their own: they take no connection of the pool of
/// `sqlx::test`, which shares 20 connections with the other tests.
#[sqlx::test(migrations = false)]
async fn the_test_server_uses_a_pool_of_its_own(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let owner = app.user().await;

    // when
    app.create_project(owner, "alpha").await.unwrap();

    // then
    assert!(pool.size() <= 1, "{} connections", pool.size());
    assert!(app.pool.size() >= 1);
}

/// The pool of the test server closes when sqlx closes the pool of the test, so sqlx can drop the
/// test database.
#[sqlx::test(migrations = false)]
async fn the_test_server_pool_closes_with_the_pool_of_the_test(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let owner = app.user().await;
    app.create_project(owner, "alpha").await.unwrap();

    // when
    pool.close().await;

    // then
    timeout(Duration::from_secs(5), app.pool.close_event())
        .await
        .expect("the pool of the server closes");
    assert!(app.pool.is_closed());
}
