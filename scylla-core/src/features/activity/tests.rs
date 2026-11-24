//! Unit tests of the feed that the API cannot show: the delete of the old rows, and the limit of
//! the parameters.

use scylla_domain::activity::{Kind, NewActivity, Place, MAX_PARAMS, RETENTION};
use sqlx::PgPool;

use super::{record, service};

/// The delete takes the rows older than `RETENTION`, and keeps the others.
#[sqlx::test(migrator = "crate::infra::db::MIGRATOR")]
async fn purge_deletes_the_old_rows(pool: PgPool) {
    // given
    let (user, _, project, _) = crate::features::projects::jobs::tests::place(&pool).await;
    let mut tx = pool.begin().await.unwrap();
    for name in ["old", "new"] {
        let row = NewActivity::new(Kind::SecretCreated, Place::Project(project))
            .actor(user.id)
            .param("name", name);
        record::record(&mut tx, row).await.unwrap();
    }
    tx.commit().await.unwrap();
    let days = i32::try_from(RETENTION.whole_days()).unwrap() + 1;
    sqlx::query!(
        "UPDATE activity SET created_at = now() - make_interval(days => $1)
         WHERE params->>'name' = 'old'",
        days,
    )
    .execute(&pool)
    .await
    .unwrap();

    // when
    let deleted = service::purge(&pool).await.unwrap();

    // then
    let left = sqlx::query_scalar!(r#"SELECT params->>'name' AS "name!" FROM activity"#)
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!((deleted, left), (1, vec!["new".to_owned()]));
}

/// A row with parameters above `MAX_PARAMS` is refused, and a rollback of its change removes a
/// row.
#[sqlx::test(migrator = "crate::infra::db::MIGRATOR")]
async fn record_keeps_the_rows_in_the_transaction(pool: PgPool) {
    // given
    let (user, _, project, _) = crate::features::projects::jobs::tests::place(&pool).await;
    let row = || NewActivity::new(Kind::SecretCreated, Place::Project(project)).actor(user.id);
    let large = row().param("name", "x".repeat(MAX_PARAMS));

    // when
    let mut tx = pool.begin().await.unwrap();
    let refused = record::record(&mut tx, large).await;
    record::record(&mut tx, row()).await.unwrap();
    tx.rollback().await.unwrap();

    // then
    let count = sqlx::query_scalar!(r#"SELECT count(*) AS "count!" FROM activity"#)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(refused.is_err());
    assert_eq!(count, 0);
}
