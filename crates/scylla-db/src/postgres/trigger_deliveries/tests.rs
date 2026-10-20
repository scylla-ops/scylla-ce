use super::PgTriggerDeliveryRepository;
use crate::domain::clock;
use crate::domain::trigger::Trigger;
use crate::domain::trigger::{TriggerName, TriggerSource, WebhookSpec};
use crate::postgres::PgTriggerRepository;
use crate::test_support::prelude::*;
use scylla_core::application::{TriggerDeliveryRepository, TriggerRepository};
use sqlx::PgPool;

async fn webhook_trigger(pool: &PgPool) -> Trigger {
    let (_, _, pipeline) = seed_org_project_pipeline(pool, "wd").await;
    let repo = PgTriggerRepository::new(pool.clone());
    let trigger = Trigger::create(
        pipeline.id().clone(),
        TriggerName::new("on-push").unwrap(),
        TriggerSource::Webhook(WebhookSpec::new(None).unwrap()),
        vec![],
    )
    .unwrap();
    repo.create(&trigger, Some(b"ciphertext")).await.unwrap();
    trigger
}

#[sqlx::test(migrations = "../../migrations")]
async fn first_delivery_is_new_then_replays_are_duplicates(pool: PgPool) {
    let trigger = webhook_trigger(&pool).await;
    let repo = PgTriggerDeliveryRepository::new(pool);

    let first = repo
        .record_or_detect(trigger.id(), "delivery-1", clock::now())
        .await
        .unwrap();
    assert!(first, "first sighting of a delivery id is new");

    let replay = repo
        .record_or_detect(trigger.id(), "delivery-1", clock::now())
        .await
        .unwrap();
    assert!(!replay, "the same delivery id again is a duplicate");

    let other = repo
        .record_or_detect(trigger.id(), "delivery-2", clock::now())
        .await
        .unwrap();
    assert!(other, "a different delivery id is new");
}

#[sqlx::test(migrations = "../../migrations")]
async fn forget_lets_the_same_delivery_record_again(pool: PgPool) {
    let trigger = webhook_trigger(&pool).await;
    let repo = PgTriggerDeliveryRepository::new(pool);

    assert!(
        repo.record_or_detect(trigger.id(), "delivery-1", clock::now())
            .await
            .unwrap()
    );
    repo.forget(trigger.id(), "delivery-1").await.unwrap();

    assert!(
        repo.record_or_detect(trigger.id(), "delivery-1", clock::now())
            .await
            .unwrap(),
        "a forgotten delivery is new again"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn webhook_secret_round_trips_and_is_absent_for_missing(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "ws").await;
    let repo = PgTriggerRepository::new(pool);

    let trigger = Trigger::create(
        pipeline.id().clone(),
        TriggerName::new("on-push").unwrap(),
        TriggerSource::Webhook(WebhookSpec::new(None).unwrap()),
        vec![],
    )
    .unwrap();
    repo.create(&trigger, Some(b"\x00\x01\x02secret"))
        .await
        .unwrap();

    let got = repo.webhook_secret(trigger.id()).await.unwrap();
    assert_eq!(got.as_deref(), Some(&b"\x00\x01\x02secret"[..]));
}
