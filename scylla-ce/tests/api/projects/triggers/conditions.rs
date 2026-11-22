//! The tests of the conditions of a webhook trigger: a delivery that does not match them makes no
//! job and shows as skipped, a fire now does not read them, and a bad condition is refused.

use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, KeyInit, Mac};
use prost_types::Timestamp;
use scylla_domain::identity::actors::ActorId;
use scylla_proto::integrations::v1::GetDeliveryStatsRequest;
use scylla_proto::pipelines::v1::update_trigger_request::Rules;
use scylla_proto::pipelines::v1::{
    trigger, webhook_condition, ConditionOperator, FireStatus, GetTriggerFireStatsRequest,
    ListTriggerFiresRequest, Trigger, Webhook, WebhookCondition,
};
use scylla_testkit::{header_condition, id, pointer_condition, text, TestApp};
use sha2::Sha256;
use sqlx::PgPool;

use super::{hex, pipeline};
use crate::{app, assert_invalid_field};

/// A webhook trigger: its id, its path and its secret.
struct Hook {
    /// The id of the trigger.
    id: String,
    /// The path of the webhook.
    path: String,
    /// The secret of the signatures.
    secret: String,
}

/// Returns the conditions of a push to `main`: the event is `push` and `/ref` is
/// `refs/heads/main`.
fn push_to_main() -> Vec<WebhookCondition> {
    vec![
        header_condition("X-GitHub-Event", ConditionOperator::Is, &["push"]),
        pointer_condition("/ref", ConditionOperator::Is, &["refs/heads/main"]),
    ]
}

/// Returns the webhook of a trigger.
fn webhook(trigger: Trigger) -> Webhook {
    let Some(trigger::Kind::Webhook(webhook)) = trigger.kind else {
        panic!("a webhook trigger")
    };
    webhook
}

/// Creates a webhook trigger with conditions in a new pipeline. Returns the owner, the pipeline
/// and the trigger.
async fn hook(app: &TestApp, conditions: Vec<WebhookCondition>) -> (ActorId, String, Hook) {
    let (owner, _, pipeline) = pipeline(app).await;
    let created = app
        .create_webhook_trigger(owner, &pipeline, "push", conditions)
        .await
        .unwrap();
    let trigger = created.trigger.unwrap();
    let id = text(&trigger.id).to_owned();
    let path = webhook(trigger).path;
    let secret = created.webhook_secret;
    (owner, pipeline, Hook { id, path, secret })
}

/// Sends a signed delivery of a GitHub event with a body and a delivery key. Returns the status
/// and the body of the answer.
async fn deliver(app: &TestApp, hook: &Hook, event: &str, body: &str, key: &str) -> (u16, String) {
    let mut mac = Hmac::<Sha256>::new_from_slice(hook.secret.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    let signature = format!("sha256={}", hex(&mac.finalize().into_bytes()));
    let response = reqwest::Client::new()
        .post(format!("http://{}{}", app.addr, hook.path))
        .header("x-hub-signature-256", signature)
        .header("x-github-delivery", key)
        .header("x-github-event", event)
        .header("content-type", "application/json")
        .body(body.to_owned())
        .send()
        .await
        .unwrap();
    (response.status().as_u16(), response.text().await.unwrap())
}

/// Returns the time one hour ago, in seconds since 1970.
fn an_hour_ago() -> Timestamp {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let seconds = i64::try_from(elapsed.as_secs()).unwrap() - 3_600;
    Timestamp { seconds, nanos: 0 }
}

/// A delivery that matches each condition starts a job. Another branch, another event and a
/// body that is not JSON get 200 with the reason and no job, and show as skipped: in the last
/// result of the trigger, in the history and in the counts, and not as failed.
#[sqlx::test(migrations = false)]
async fn a_webhook_skips_a_delivery_that_does_not_match_the_conditions(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, pipeline, hook) = hook(&app, push_to_main()).await;
    let workspace = app.workspace_of(owner).await.to_string();
    let (main, dev) = (
        r#"{"ref":"refs/heads/main"}"#,
        r#"{"ref":"refs/heads/dev"}"#,
    );

    // when
    let fired = deliver(&app, &hook, "push", main, "delivery-1").await;
    let other_branch = deliver(&app, &hook, "push", dev, "delivery-2").await;
    let other_event = deliver(&app, &hook, "issues", main, "delivery-3").await;
    let not_json = deliver(&app, &hook, "push", "ref=main", "delivery-4").await;

    // then
    assert_eq!(fired.0, 202, "{}", fired.1);
    assert_eq!((other_branch.0, other_event.0, not_json.0), (200, 200, 200));
    assert!(other_branch.1.contains("No match: /ref is refs/heads/dev"));
    assert!(other_event.1.contains("No match: X-GitHub-Event is issues"));
    assert!(not_json.1.contains("No match: the body is not JSON"));
    let jobs = app.list_pipeline_jobs(owner, &pipeline, 0, "").await;
    assert_eq!(jobs.unwrap().jobs.len(), 1);
    let read = app.get_trigger(owner, &hook.id).await.unwrap();
    assert_eq!(read.last_status(), FireStatus::Skipped);
    assert_eq!(read.last_error, "No match: the body is not JSON");
    let skipped = ListTriggerFiresRequest {
        workspace_id: Some(id(&workspace)),
        statuses: vec![FireStatus::Skipped.into()],
        ..ListTriggerFiresRequest::default()
    };
    let skipped = app.list_trigger_fires(owner, skipped).await.unwrap().fires;
    let reasons: Vec<_> = skipped.iter().map(|fire| fire.error.as_str()).collect();
    assert_eq!(
        reasons,
        vec![
            "No match: the body is not JSON",
            "No match: X-GitHub-Event is issues",
            "No match: /ref is refs/heads/dev",
        ]
    );
    assert!(skipped.iter().all(|fire| fire.job_id.is_none()));
    let stats = GetTriggerFireStatsRequest {
        workspace_id: Some(id(&workspace)),
        from: Some(an_hour_ago()),
        ..GetTriggerFireStatsRequest::default()
    };
    let stats = app.get_trigger_fire_stats(owner, stats).await.unwrap();
    let totals = stats.totals.unwrap();
    let figures = (totals.total, totals.ok, totals.skipped, totals.failed_fires);
    assert_eq!(figures, (4, 1, 3, 0));
    let deliveries = GetDeliveryStatsRequest {
        workspace_id: Some(id(&workspace)),
        from: Some(an_hour_ago()),
        ..GetDeliveryStatsRequest::default()
    };
    let deliveries = app.get_delivery_stats(owner, deliveries).await.unwrap();
    let figures = deliveries.triggers[0].figures.unwrap();
    assert_eq!((figures.delivered, figures.failed), (4, 0));
}

/// A fire now starts a job also when the conditions would skip a delivery.
#[sqlx::test(migrations = false)]
async fn fire_trigger_now_does_not_read_the_conditions(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _, hook) = hook(&app, push_to_main()).await;

    // when
    let job = app.fire_trigger_now(owner, &hook.id).await.unwrap();

    // then
    assert_eq!(text(&job.trigger_id), hook.id);
    let read = app.get_trigger(owner, &hook.id).await.unwrap();
    assert_eq!(read.last_status(), FireStatus::Ok);
}

/// A condition with no source, no operator, a pointer with no `/`, an empty header name, an
/// empty value or too many values, and more than 32 conditions give INVALID_ARGUMENT on the
/// field.
#[sqlx::test(migrations = false)]
async fn create_trigger_refuses_a_bad_condition(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _, pipeline) = pipeline(&app).await;
    let is = ConditionOperator::Is;
    let no_source = WebhookCondition {
        source: None,
        ..header_condition("X-GitHub-Event", is, &["push"])
    };
    let no_operator = header_condition("X-GitHub-Event", ConditionOperator::Unspecified, &["a"]);
    let cases = [
        (vec![no_source], "webhook.conditions[0].source"),
        (vec![no_operator], "webhook.conditions[0].operator"),
        (
            vec![pointer_condition("ref", is, &["refs/heads/main"])],
            "webhook.conditions[0].json_pointer",
        ),
        (
            vec![header_condition("", is, &["push"])],
            "webhook.conditions[0].header",
        ),
        (
            vec![pointer_condition("/ref", is, &[""])],
            "webhook.conditions[0].values[0]",
        ),
        (
            vec![pointer_condition("/ref", is, &["a", "b"])],
            "webhook.conditions[0].values",
        ),
        (vec![push_to_main()[0].clone(); 33], "webhook.conditions"),
    ];

    for (conditions, field) in cases {
        // when
        let refused = app
            .create_webhook_trigger(owner, &pipeline, "push", conditions)
            .await;

        // then
        assert_invalid_field(&refused.unwrap_err(), field);
    }
}

/// An update changes the conditions of a webhook and the next deliveries follow them. A bad
/// condition, and the conditions of a webhook on a cron trigger, give INVALID_ARGUMENT.
#[sqlx::test(migrations = false)]
async fn update_trigger_changes_the_conditions(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, pipeline, hook) = hook(&app, push_to_main()).await;
    let read = app.get_trigger(owner, &hook.id).await.unwrap();
    let dev = r#"{"ref":"refs/heads/dev"}"#;
    let skipped = deliver(&app, &hook, "push", dev, "delivery-1").await;
    let any_branch = pointer_condition("/ref", ConditionOperator::Matches, &["refs/heads/*"]);
    let rules = |conditions| {
        Rules::Webhook(Webhook {
            path: String::new(),
            conditions,
        })
    };
    let cron = app
        .create_trigger(owner, &pipeline, "nightly", Some("0 3 * * *"), Vec::new())
        .await
        .unwrap();
    let cron = text(&cron.trigger.unwrap().id).to_owned();

    // when
    let changed = app
        .update_trigger_rules(owner, &hook.id, rules(vec![any_branch.clone()]))
        .await
        .unwrap();
    let fired = deliver(&app, &hook, "push", dev, "delivery-2").await;
    let bad = pointer_condition("ref", ConditionOperator::Is, &["main"]);
    let refused = app
        .update_trigger_rules(owner, &hook.id, rules(vec![bad]))
        .await;
    let on_cron = app
        .update_trigger_rules(owner, &cron, rules(vec![any_branch.clone()]))
        .await;

    // then
    assert_eq!(webhook(read).conditions, push_to_main());
    assert_eq!(skipped.0, 200, "{}", skipped.1);
    let changed = webhook(changed);
    assert_eq!(changed.conditions, vec![any_branch]);
    assert_eq!(changed.path, hook.path);
    assert!(matches!(
        changed.conditions[0].source,
        Some(webhook_condition::Source::JsonPointer(_))
    ));
    assert_eq!(fired.0, 202, "{}", fired.1);
    assert_invalid_field(&refused.unwrap_err(), "webhook.conditions[0].json_pointer");
    assert_invalid_field(&on_cron.unwrap_err(), "rules");
}
