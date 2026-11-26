//! Unit tests of the loop of the cron schedules, which the API shows only after 15 seconds.

use std::sync::Arc;
use std::time::Duration;

use scylla_domain::TimeZone;
use sqlx::PgPool;
use time::OffsetDateTime;

use super::schedule;
use crate::features::identity::accounts::AccountSettings;
use crate::features::integrations::IntegrationContext;
use crate::features::projects::no_gate::NoGate;
use crate::infra::vcs::{VcsClients, VcsOptions};
use crate::SignUpMode;

/// Returns what the loop shares with the Git integration: the pool, no hook, no agent, and the
/// public providers, with the TLS provider of the process.
fn context(pool: PgPool) -> IntegrationContext {
    crate::infra::tls::install();
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
            mail_sender: "no-reply@localhost".to_owned(),
            local_pictures: false,
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
    let next = schedule::next("*/5 * * * *", TimeZone::UTC, start).unwrap();

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
    let next = schedule::next("0 3 * * 7L", TimeZone::UTC, start);

    // then
    assert_eq!(
        next.map(OffsetDateTime::unix_timestamp),
        Some(1_795_921_200)
    );
}

/// Returns the unix times of a schedule in Europe/Paris between two unix times.
fn paris(schedule: &str, after: i64, until: i64) -> Vec<i64> {
    let zone = TimeZone::parse("Europe/Paris".to_owned()).unwrap();
    let at = |seconds| OffsetDateTime::from_unix_timestamp(seconds).unwrap();
    schedule::between(schedule, zone, at(after), at(until), 10)
        .into_iter()
        .map(OffsetDateTime::unix_timestamp)
        .collect()
}

/// On the night of the change to summer time, 02:30 does not exist in Paris: the schedule does
/// not fire that night, and fires the next night at 00:30 UTC.
#[test]
fn a_wall_time_that_summer_time_skips_does_not_fire() {
    // given
    let (after, until) = (1_774_699_200, 1_774_872_000);

    // when
    let times = paris("30 2 * * *", after, until);

    // then
    assert_eq!(times, vec![1_774_830_600]);
}

/// On the night of the change to winter time, 02:30 occurs two times in Paris: the schedule fires
/// one time, at 00:30 UTC, and the next night at 01:30 UTC.
#[test]
fn a_wall_time_that_winter_time_repeats_fires_one_time() {
    // given
    let (after, until) = (1_792_843_200, 1_793_016_000);

    // when
    let times = paris("30 2 * * *", after, until);

    // then
    assert_eq!(times, vec![1_792_888_200, 1_792_978_200]);
}

/// A due trigger fires one time with the origin `cron` and gets its next time. The history keeps
/// the fire and the cron time that the loop missed, and the activity feed gets one row for the
/// missed time. A stopped trigger does not fire.
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
        r#"SELECT count(*) AS "count!" FROM jobs
           WHERE pipeline_id = $1 AND trigger_id IS NOT NULL AND origin = 'cron'"#,
        pipeline.to_db()
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(jobs, 1);
    let fires = sqlx::query!(
        r#"SELECT source, status, job_id IS NOT NULL AS "job!" FROM trigger_fires
           WHERE pipeline_id = $1 ORDER BY due_at"#,
        pipeline.to_db()
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let fires: Vec<_> = fires
        .into_iter()
        .map(|row| (row.source, row.status, row.job))
        .collect();
    let row = |status: &str, job| ("schedule".to_owned(), status.to_owned(), job);
    assert_eq!(fires, vec![row("ok", true), row("missed", false)]);
    let missed = sqlx::query!(
        r#"SELECT params->>'name' AS "name!", params->>'count' AS "count!" FROM activity
           WHERE kind = 'trigger.time_missed' AND area = 'triggers'"#
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let missed: Vec<_> = missed
        .into_iter()
        .map(|row| (row.name, row.count))
        .collect();
    assert_eq!(missed, vec![("due".to_owned(), "1".to_owned())]);
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
