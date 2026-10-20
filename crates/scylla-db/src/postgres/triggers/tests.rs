use super::PgTriggerRepository;
use crate::domain::clock;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::pipeline::EnvKey;
use crate::domain::pipeline::Pipeline;
use crate::domain::trigger::{
    CronSpec, FireObservation, Trigger, TriggerInput, TriggerName, TriggerSource, WebhookSpec,
};
use crate::postgres::PgPipelineRepository;
use crate::test_support::prelude::*;
use chrono::{DateTime, Duration, Utc};
use scylla_core::application::{PipelineRepository, TriggerRepository};
use sqlx::PgPool;

fn cron_trigger(pipeline: &Pipeline, name: &str) -> Trigger {
    Trigger::create(
        pipeline.id().clone(),
        TriggerName::new(name).unwrap(),
        TriggerSource::Cron(CronSpec::new("0 9 * * *").unwrap()),
        vec![],
    )
    .unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn cron_trigger_round_trips_with_inputs(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "u").await;
    let repo = PgTriggerRepository::new(pool);

    let trigger = Trigger::create(
        pipeline.id().clone(),
        TriggerName::new("nightly").unwrap(),
        TriggerSource::Cron(CronSpec::new("0 9 * * 1-5").unwrap()),
        vec![TriggerInput::literal(EnvKey::new("RUN_MODE").unwrap(), "nightly").unwrap()],
    )
    .unwrap();
    repo.create(&trigger, None).await.unwrap();

    let found = repo.find_by_id(trigger.id()).await.unwrap();
    assert_eq!(found.name().as_str(), "nightly");
    assert!(found.is_enabled());
    assert_eq!(found.inputs().len(), 1);
    match found.source() {
        TriggerSource::Cron(c) => assert_eq!(c.expression(), "0 9 * * 1-5"),
        TriggerSource::Webhook(_) => panic!("expected cron source"),
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn webhook_trigger_round_trips_with_json_pointer(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "w").await;
    let repo = PgTriggerRepository::new(pool);

    let trigger = Trigger::create(
        pipeline.id().clone(),
        TriggerName::new("on-push").unwrap(),
        TriggerSource::Webhook(WebhookSpec::new(Some("X-Hub-Signature-256".into())).unwrap()),
        vec![TriggerInput::json_pointer(EnvKey::new("GIT_COMMIT").unwrap(), "/after").unwrap()],
    )
    .unwrap();
    repo.create(&trigger, None).await.unwrap();

    let found = repo.find_by_id(trigger.id()).await.unwrap();
    match found.source() {
        TriggerSource::Webhook(w) => {
            assert_eq!(w.signature_header(), Some("X-Hub-Signature-256"));
        }
        TriggerSource::Cron(_) => panic!("expected webhook source"),
    }
    assert_eq!(found.inputs().len(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_by_pipeline_filters(pool: PgPool) {
    let org = seed_org(&pool, "acme").await;
    let project = seed_project(&pool, &org, "rocket").await;
    let pipelines = PgPipelineRepository::new(pool.clone());
    let pa = pipeline(&project);
    let pb = pipeline(&project);
    pipelines.create(&pa).await.unwrap();
    pipelines.create(&pb).await.unwrap();
    let repo = PgTriggerRepository::new(pool);

    repo.create(&cron_trigger(&pa, "a"), None).await.unwrap();
    repo.create(&cron_trigger(&pa, "b"), None).await.unwrap();
    repo.create(&cron_trigger(&pb, "c"), None).await.unwrap();

    assert_eq!(repo.list_by_pipeline(pa.id()).await.unwrap().len(), 2);
    assert_eq!(repo.list_by_pipeline(pb.id()).await.unwrap().len(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn update_persists_changes_and_bumps_the_version(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "up").await;
    let repo = PgTriggerRepository::new(pool);

    let mut trigger = cron_trigger(&pipeline, "nightly");
    repo.create(&trigger, None).await.unwrap();

    trigger
        .update(
            TriggerName::new("midnight").unwrap(),
            TriggerSource::Cron(CronSpec::new("0 0 * * *").unwrap()),
            vec![],
        )
        .unwrap();
    let updated = repo.update(&trigger).await.unwrap();

    assert_eq!(updated.version(), 1);
    let found = repo.find_by_id(trigger.id()).await.unwrap();
    assert_eq!(found.name().as_str(), "midnight");
    assert_eq!(found.version(), 1);
    match found.source() {
        TriggerSource::Cron(c) => assert_eq!(c.expression(), "0 0 * * *"),
        TriggerSource::Webhook(_) => panic!("expected cron source"),
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_stale_edit_or_delete_is_refused(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "st").await;
    let repo = PgTriggerRepository::new(pool);
    let read = cron_trigger(&pipeline, "nightly");
    repo.create(&read, None).await.unwrap();

    let mut disabled = read.clone();
    disabled.disable();
    let fresh = repo.update(&disabled).await.unwrap();

    let mut renamed = read.clone();
    renamed
        .update(
            TriggerName::new("hourly").unwrap(),
            read.source().clone(),
            vec![],
        )
        .unwrap();
    let update = repo.update(&renamed).await;
    let delete = repo.delete(&read).await;

    assert!(matches!(update, Err(DomainError::Stale(_))), "{update:?}");
    assert!(matches!(delete, Err(DomainError::Stale(_))), "{delete:?}");
    let stored = repo.find_by_id(read.id()).await.unwrap();
    assert!(!stored.is_enabled());
    assert_eq!(stored.name().as_str(), "nightly");

    repo.delete(&fresh).await.unwrap();
    assert!(matches!(
        repo.delete(&fresh).await,
        Err(DomainError::NotFound(_))
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn set_enabled_persists(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "en").await;
    let repo = PgTriggerRepository::new(pool);

    let mut trigger = cron_trigger(&pipeline, "nightly");
    repo.create(&trigger, None).await.unwrap();

    trigger.disable();
    repo.update(&trigger).await.unwrap();

    assert!(!repo.find_by_id(trigger.id()).await.unwrap().is_enabled());
}

#[sqlx::test(migrations = "../../migrations")]
async fn record_fire_writes_only_the_observation(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "rf").await;
    let repo = PgTriggerRepository::new(pool);
    let mut trigger = cron_trigger(&pipeline, "nightly");
    repo.create(&trigger, None).await.unwrap();
    trigger.disable();
    let disabled = repo.update(&trigger).await.unwrap();

    let observation = FireObservation {
        fired_at: clock::now(),
        status: "ok".to_owned(),
    };
    repo.record_fire(trigger.id(), &observation).await.unwrap();

    let reloaded = repo.find_by_id(trigger.id()).await.unwrap();
    assert!(!reloaded.is_enabled());
    assert_eq!(reloaded.last_status(), Some("ok"));
    assert!(reloaded.last_fired_at().is_some());
    assert_eq!(reloaded.updated_at(), disabled.updated_at());
    assert_eq!(reloaded.version(), disabled.version());
}

#[sqlx::test(migrations = "../../migrations")]
async fn update_keeps_the_last_fire_recorded_after_the_read(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "kf").await;
    let repo = PgTriggerRepository::new(pool);
    let mut trigger = cron_trigger(&pipeline, "nightly");
    repo.create(&trigger, None).await.unwrap();

    let observation = FireObservation {
        fired_at: clock::now(),
        status: "error".to_owned(),
    };
    repo.record_fire(trigger.id(), &observation).await.unwrap();
    trigger
        .update(
            TriggerName::new("hourly").unwrap(),
            trigger.source().clone(),
            vec![],
        )
        .unwrap();
    repo.update(&trigger).await.unwrap();

    let reloaded = repo.find_by_id(trigger.id()).await.unwrap();
    assert_eq!(reloaded.name().as_str(), "hourly");
    assert!(reloaded.is_enabled());
    assert_eq!(reloaded.last_status(), Some("error"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn duplicate_name_in_pipeline_conflicts(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "dup").await;
    let repo = PgTriggerRepository::new(pool);
    let taken = "Trigger name already exists on this pipeline";

    repo.create(&cron_trigger(&pipeline, "dup"), None)
        .await
        .unwrap();
    let create = repo.create(&cron_trigger(&pipeline, "dup"), None).await;

    let mut other = cron_trigger(&pipeline, "other");
    repo.create(&other, None).await.unwrap();
    other
        .update(
            TriggerName::new("dup").unwrap(),
            other.source().clone(),
            vec![],
        )
        .unwrap();
    let rename = repo.update(&other).await;

    assert!(
        matches!(&create, Err(DomainError::Conflict(m)) if m == taken),
        "{create:?}"
    );
    assert!(
        matches!(&rename, Err(DomainError::Conflict(m)) if m == taken),
        "{rename:?}"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn cascade_pipeline_delete_removes_triggers(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "cas").await;
    let repo = PgTriggerRepository::new(pool.clone());

    let trigger = cron_trigger(&pipeline, "x");
    repo.create(&trigger, None).await.unwrap();

    PgPipelineRepository::new(pool)
        .delete(&pipeline)
        .await
        .unwrap();

    assert!(matches!(
        repo.find_by_id(trigger.id()).await,
        Err(DomainError::NotFound(_)),
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn delete_then_find_returns_not_found(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "del").await;
    let repo = PgTriggerRepository::new(pool);

    let trigger = cron_trigger(&pipeline, "x");
    repo.create(&trigger, None).await.unwrap();

    repo.delete(&trigger).await.unwrap();
    assert!(matches!(
        repo.find_by_id(trigger.id()).await,
        Err(DomainError::NotFound(_)),
    ));
}

async fn cron_trigger_at(
    repo: &PgTriggerRepository,
    pipeline: &Pipeline,
    name: &str,
    next_fire_at: Option<DateTime<Utc>>,
) -> Trigger {
    let mut trigger = cron_trigger(pipeline, name);
    if let Some(next) = next_fire_at {
        trigger.set_next_fire_at(Some(next));
    }
    repo.create(&trigger, None).await.unwrap();
    trigger
}

#[sqlx::test(migrations = "../../migrations")]
async fn seed_cron_schedules_only_enabled_cron_without_next_fire(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "seed").await;
    let repo = PgTriggerRepository::new(pool);
    let now = clock::now();
    let first = now + Duration::hours(1);

    let fresh = cron_trigger_at(&repo, &pipeline, "fresh", None).await;
    let scheduled = cron_trigger_at(&repo, &pipeline, "scheduled", Some(now)).await;
    let mut disabled = cron_trigger(&pipeline, "disabled");
    disabled.disable();
    repo.create(&disabled, None).await.unwrap();
    let webhook = Trigger::create(
        pipeline.id().clone(),
        TriggerName::new("hook").unwrap(),
        TriggerSource::Webhook(WebhookSpec::new(None).unwrap()),
        vec![],
    )
    .unwrap();
    repo.create(&webhook, None).await.unwrap();
    let before = repo.find_by_id(fresh.id()).await.unwrap();

    let compute = |_: &Trigger| -> DomainResult<DateTime<Utc>> { Ok(first) };
    let seeded = repo.seed_cron(&compute).await.unwrap();

    assert_eq!(seeded.len(), 1);
    assert_eq!(seeded[0].id(), fresh.id());
    let reloaded = repo.find_by_id(fresh.id()).await.unwrap();
    assert_eq!(reloaded.next_fire_at(), Some(first));
    assert_eq!(reloaded.updated_at(), before.updated_at());
    assert_eq!(reloaded.version(), before.version());
    assert_ne!(
        repo.find_by_id(scheduled.id())
            .await
            .unwrap()
            .next_fire_at(),
        Some(first)
    );
    assert!(repo.seed_cron(&compute).await.unwrap().is_empty());
}

#[sqlx::test(migrations = "../../migrations")]
async fn claim_due_cron_claims_due_advances_and_excludes_others(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "claim").await;
    let repo = PgTriggerRepository::new(pool);
    let now = clock::now();
    let past = now - Duration::minutes(1);
    let future = now + Duration::minutes(30);
    let advanced = now + Duration::hours(1);

    let due = cron_trigger_at(&repo, &pipeline, "due", Some(past)).await;
    cron_trigger_at(&repo, &pipeline, "later", Some(future)).await;
    let mut disabled = cron_trigger(&pipeline, "off");
    disabled.disable();
    repo.create(&disabled, None).await.unwrap();

    let compute = |_t: &Trigger| -> DomainResult<DateTime<Utc>> { Ok(advanced) };

    let claimed = repo.claim_due_cron(now, 10, &compute).await.unwrap();
    assert_eq!(claimed.len(), 1, "only the enabled, due trigger is claimed");
    assert_eq!(claimed[0].id(), due.id());
    assert_eq!(claimed[0].next_fire_at(), Some(past));

    let reloaded = repo.find_by_id(due.id()).await.unwrap();
    assert_eq!(reloaded.next_fire_at(), Some(advanced));
    assert_eq!(reloaded.updated_at(), claimed[0].updated_at());
    assert_eq!(reloaded.version(), claimed[0].version());

    assert!(
        repo.claim_due_cron(now, 10, &compute)
            .await
            .unwrap()
            .is_empty()
    );
}
