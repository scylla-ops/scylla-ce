//! Unit tests of the loop of the cron schedules, which the API shows only after 15 seconds.

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use time::OffsetDateTime;

use super::schedule;
use crate::features::identity::accounts::AccountSettings;
use crate::features::integrations::IntegrationContext;
use crate::features::projects::no_gate::NoGate;
use crate::infra::vcs::{VcsClients, VcsOptions};
use crate::SignUpMode;

/// Returns what the loop shares with the Git integration: the pool, no hook, no agent, and the
/// public providers.
fn context(pool: PgPool) -> IntegrationContext {
    let settings = VcsOptions::default();
    IntegrationContext {
        pool,
        gate: Arc::new(NoGate),
        hooks: Arc::default(),
        registry: Arc::default(),
        sealer: None,
        clients: VcsClients::new(&settings).unwrap(),
        settings: Arc::new(settings),
        urls: Arc::new(AccountSettings {
            api_url: "http://127.0.0.1".to_owned(),
            app_url: "http://127.0.0.1".to_owned(),
            sign_up_mode: SignUpMode::Closed,
            session_ttl: Duration::from_secs(60),
            web_origins: Vec::new(),
            trust_forwarded: false,
        }),
        wake: Arc::default(),
    }
}

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

/// A schedule on the last Sunday of the month, written as `7L`, gives that Sunday.
#[test]
fn a_schedule_on_the_last_sunday_gives_its_next_time() {
    // given
    let start = OffsetDateTime::from_unix_timestamp(1_793_491_200).unwrap();

    // when
    let next = schedule::next("0 3 * * 7L", start);

    // then
    assert_eq!(
        next.map(OffsetDateTime::unix_timestamp),
        Some(1_795_921_200)
    );
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
    let vcs = context(pool.clone());
    let first = schedule::tick(&vcs).await.unwrap();
    let second = schedule::tick(&vcs).await.unwrap();

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

/// A due trigger of an archived pipeline does not fire, and it gets its next time.
#[sqlx::test(migrator = "crate::infra::db::MIGRATOR")]
async fn tick_skips_the_triggers_of_an_archived_pipeline(pool: PgPool) {
    // given
    let (_, _, _, pipeline) = crate::features::projects::jobs::tests::place(&pool).await;
    sqlx::query!(
        "UPDATE pipelines SET archived_at = now() WHERE id = $1",
        pipeline.to_db()
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query!(
        "INSERT INTO triggers (id, pipeline_id, project_id, workspace_id, created_by, name, kind,
                               schedule, inputs, next_fire_at)
         SELECT gen_random_uuid(), p.id, p.project_id, p.workspace_id, p.created_by, 'due', 'cron',
                '* * * * *', '[]', now() - interval '1 minute'
         FROM pipelines p WHERE p.id = $1",
        pipeline.to_db(),
    )
    .execute(&pool)
    .await
    .unwrap();

    // when
    let fired = schedule::tick(&context(pool.clone())).await.unwrap();

    // then
    assert_eq!(fired, 0);
    let next = sqlx::query_scalar!(
        r#"SELECT next_fire_at > now() AS "later!" FROM triggers WHERE pipeline_id = $1"#,
        pipeline.to_db()
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(next);
}
