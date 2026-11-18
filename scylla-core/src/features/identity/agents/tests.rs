//! Unit tests of the samples of the agents that the API cannot show: the purge after 7 days.

use scylla_domain::identity::agents::{Sample, SAMPLE_RETENTION};
use scylla_domain::identity::apps::{AppId, NewApp};
use scylla_domain::Name;
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};

use super::samples;
use crate::features::identity::apps;
use crate::features::projects::jobs::tests::place;

/// Creates an agent in the workspace of a new user.
async fn agent(pool: &PgPool) -> AppId {
    let (user, workspace, _, _) = place(pool).await;
    let new = NewApp {
        workspace_id: workspace,
        created_by: user.id,
        name: Name::parse("sampled".to_owned()).unwrap(),
        runner: true,
    };
    apps::create(pool, new).await.unwrap().0.id
}

/// The purge deletes the samples of more than 7 days, and keeps the others.
#[sqlx::test(migrator = "crate::infra::db::MIGRATOR")]
async fn purge_deletes_the_samples_of_more_than_7_days(pool: PgPool) {
    // given
    let agent = agent(&pool).await;
    let old = OffsetDateTime::now_utc() - SAMPLE_RETENTION - Duration::minutes(1);
    sqlx::query!(
        "INSERT INTO agent_samples (agent_id, at, cpu_percent, memory_used_bytes)
         VALUES ($1, $2, 10, 100)",
        agent.to_db(),
        old,
    )
    .execute(&pool)
    .await
    .unwrap();
    let sample = Sample::parse(20.0, 200).unwrap();
    samples::record(&pool, agent, sample).await.unwrap();

    // when
    let purged = samples::purge(&pool).await.unwrap();

    // then
    let left: i64 = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM agent_samples WHERE agent_id = $1"#,
        agent.to_db(),
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((purged, left), (1, 1));
    let last = samples::last(&pool, &[agent]).await.unwrap();
    assert_eq!(last.get(&agent).map(|last| last.sample), Some(sample));
}
