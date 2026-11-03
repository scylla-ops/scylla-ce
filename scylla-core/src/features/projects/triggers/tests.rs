//! Unit tests of the loop of the cron schedules, which the API shows only after 15 seconds.

use scylla_extension::Hooks;
use sqlx::PgPool;
use time::OffsetDateTime;

use super::schedule;
use crate::features::identity::agents::Registry;

/// The next time of a schedule is after the start, and a bad schedule is refused.
#[test]
fn a_schedule_gives_its_next_time() {
    // given
    let start = OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();

    // when
    let next = schedule::next("*/5 * * * *", start).unwrap();

    // then
    assert!(next > start && next.minute().is_multiple_of(5) && next.second() == 0);
    assert!(schedule::check("* * * *").is_err());
    assert!(schedule::check("@hourly").is_err());
    assert!(schedule::check("0 3 * * 1-5").is_ok());
}

/// A due trigger fires one time and gets its next time. A stopped trigger does not fire.
#[sqlx::test(migrator = "crate::infra::db::MIGRATOR")]
async fn tick_fires_the_due_triggers(pool: PgPool) {
    // given
    let (_, _, _, pipeline) = crate::features::projects::jobs::tests::place(&pool).await;
    sqlx::query!(
        "UPDATE pipelines SET nodes = $2 WHERE id = $1",
        pipeline.to_db(),
        serde_json::json!([{ "id": "build", "needs": [], "step": { "kind": "exec", "argv": ["make"] },
            "env": [], "timeout_seconds": 60, "retries": 0, "working_dir": null, "image": null }]),
    )
    .execute(&pool)
    .await
    .unwrap();
    for (name, enabled) in [("due", true), ("stopped", false)] {
        sqlx::query!(
            "INSERT INTO triggers (id, pipeline_id, project_id, workspace_id, created_by, name, kind,
                                   schedule, inputs, enabled, next_fire_at)
             SELECT gen_random_uuid(), p.id, p.project_id, p.workspace_id, p.created_by, $2, 'cron',
                    '* * * * *', '[]', $3, now() - interval '1 minute'
             FROM pipelines p WHERE p.id = $1",
            pipeline.to_db(),
            name,
            enabled,
        )
        .execute(&pool)
        .await
        .unwrap();
    }

    // when
    let first = schedule::tick(&pool, &Hooks::default(), &Registry::default())
        .await
        .unwrap();
    let second = schedule::tick(&pool, &Hooks::default(), &Registry::default())
        .await
        .unwrap();

    // then
    assert_eq!((first, second), (1, 0));
    let jobs = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM jobs WHERE pipeline_id = $1 AND trigger_id IS NOT NULL"#,
        pipeline.to_db()
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(jobs, 1);
}
