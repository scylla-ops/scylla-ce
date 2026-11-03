//! The tests of the triggers: the schedules, the inputs, the roles, a fire now, and the public
//! webhook route with its signature, its deliveries and its limits.

use hmac::{Hmac, KeyInit, Mac};
use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::Project;
use scylla_proto::pipelines::v1::{env_var, trigger, Job};
use scylla_testkit::{exec_node, literal_input, pointer_input, text, TestApp};
use sha2::Sha256;
use sqlx::PgPool;
use tonic::Code;
use tonic_types::StatusExt;

use crate::{app, assert_code, assert_invalid_field};

/// Creates a user, a project and a pipeline of one node. Returns the user, the project id and
/// the pipeline id.
async fn pipeline(app: &TestApp) -> (ActorId, String, String) {
    let owner = app.user().await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let project = text(&project.id).to_owned();
    let pipeline = app
        .create_pipeline(owner, &project, vec![exec_node("build", &[])])
        .await
        .unwrap();
    (owner, project, text(&pipeline.id).to_owned())
}

/// Sends a webhook request with a body, signed with `secret`, and a delivery key.
async fn post(app: &TestApp, path: &str, body: &str, secret: &str, key: &str) -> (u16, String) {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    let signature = format!("sha256={}", hex(&mac.finalize().into_bytes()));
    let response = reqwest::Client::new()
        .post(format!("http://{}{path}", app.addr))
        .header("x-hub-signature-256", signature)
        .header("x-github-delivery", key)
        .header("content-type", "application/json")
        .body(body.to_owned())
        .send()
        .await
        .unwrap();
    (response.status().as_u16(), response.text().await.unwrap())
}

/// Returns the hex of bytes.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Returns the value of a variable of the first node of a job.
fn var(job: &Job, name: &str) -> Option<String> {
    job.nodes[0]
        .env
        .iter()
        .find(|var| var.name == name)
        .and_then(|var| match &var.value {
            Some(env_var::Value::Literal(text)) => Some(text.clone()),
            _ => None,
        })
}

/// A cron trigger gets its next time, changes its schedule, stops and starts, and goes. A bad
/// schedule and a pointer of a cron trigger are refused.
#[sqlx::test(migrations = false)]
async fn a_cron_trigger_follows_its_schedule(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _, pipeline) = pipeline(&app).await;
    let bad = app
        .create_trigger(owner, &pipeline, "nightly", Some("* * *"), Vec::new())
        .await;
    let pointer = vec![pointer_input("SHA", "/after")];
    let cron_pointer = app
        .create_trigger(owner, &pipeline, "nightly", Some("0 3 * * *"), pointer)
        .await;

    // when
    let created = app
        .create_trigger(
            owner,
            &pipeline,
            "nightly",
            Some("0 3 * * *"),
            vec![literal_input("MODE", "full")],
        )
        .await
        .unwrap();
    let id = text(&created.trigger.as_ref().unwrap().id).to_owned();
    let same_name = app
        .create_trigger(owner, &pipeline, "nightly", Some("0 4 * * *"), Vec::new())
        .await;
    let changed = app
        .update_trigger(owner, &id, Some("*/10 * * * *"), None)
        .await
        .unwrap();
    let stopped = app.set_trigger_enabled(owner, &id, false).await.unwrap();
    let listed = app.list_pipeline_triggers(owner, &pipeline).await.unwrap();
    app.delete_trigger(owner, &id).await.unwrap();

    // then
    assert_invalid_field(&bad.unwrap_err(), "cron.schedule");
    assert_invalid_field(&cron_pointer.unwrap_err(), "inputs[0].json_pointer");
    let status = same_name.unwrap_err();
    assert_eq!(
        status.get_details_error_info().unwrap().reason,
        "TRIGGER_NAME_IN_USE"
    );
    assert!(created.webhook_secret.is_empty());
    assert!(created.trigger.unwrap().next_fire_at.is_some());
    let Some(trigger::Kind::Cron(cron)) = changed.kind else {
        panic!("a cron trigger")
    };
    assert_eq!(cron.schedule, "*/10 * * * *");
    assert!(!stopped.enabled);
    assert_eq!(listed.triggers.len(), 1);
    assert_code(app.get_trigger(owner, &id).await, Code::PermissionDenied);
}

/// A fire now starts a job with the inputs, as the runner of the triggers of the workspace.
#[sqlx::test(migrations = false)]
async fn fire_trigger_now_starts_a_job_as_the_runner(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _, pipeline) = pipeline(&app).await;
    let inputs = vec![literal_input("MODE", "full")];
    let created = app
        .create_trigger(owner, &pipeline, "manual", Some("0 3 * * *"), inputs)
        .await
        .unwrap();
    let id = text(&created.trigger.unwrap().id).to_owned();

    // when
    let job = app.fire_trigger_now(owner, &id).await.unwrap();
    let again = app.fire_trigger_now(owner, &id).await.unwrap();

    // then
    assert_eq!(text(&job.trigger_id), id);
    assert_eq!(var(&job, "MODE").as_deref(), Some("full"));
    assert_ne!(text(&job.created_by), owner.to_string());
    assert_eq!(job.created_by, again.created_by);
    let read = app.get_trigger(owner, &id).await.unwrap();
    assert_eq!(
        read.last_status,
        scylla_proto::pipelines::v1::FireStatus::Ok as i32
    );
}

/// A signed delivery starts a job with the values of the body. A second delivery of one key,
/// a bad signature, a ping, a stopped trigger and a body with no value give no job.
#[sqlx::test(migrations = false)]
async fn a_webhook_needs_its_signature(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _, pipeline) = pipeline(&app).await;
    let inputs = vec![
        pointer_input("SHA", "/after"),
        literal_input("MODE", "push"),
    ];
    let created = app
        .create_trigger(owner, &pipeline, "push", None, inputs)
        .await
        .unwrap();
    let (trigger, secret) = (created.trigger.unwrap(), created.webhook_secret);
    let Some(trigger::Kind::Webhook(webhook)) = trigger.kind else {
        panic!("a webhook")
    };
    let body = r#"{"after":"abc123","ref":"refs/heads/main"}"#;

    // when
    let fired = post(&app, &webhook.path, body, &secret, "delivery-1").await;
    let duplicate = post(&app, &webhook.path, body, &secret, "delivery-1").await;
    let forged = post(&app, &webhook.path, body, "not the secret", "delivery-2").await;
    let missing = post(&app, &webhook.path, r#"{"ref":"x"}"#, &secret, "delivery-3").await;
    let retried = post(&app, &webhook.path, body, &secret, "delivery-3").await;
    let unknown = post(
        &app,
        "/webhooks/00000000-0000-0000-0000-000000000000",
        body,
        &secret,
        "x",
    )
    .await;
    app.set_trigger_enabled(owner, text(&trigger.id), false)
        .await
        .unwrap();
    let stopped = post(&app, &webhook.path, body, &secret, "delivery-4").await;

    // then
    assert_eq!(fired.0, 202, "{}", fired.1);
    assert_eq!(
        (duplicate.0, forged.0, missing.0, retried.0),
        (200, 401, 422, 202)
    );
    assert_eq!((unknown.0, stopped.0), (404, 404));
    let jobs = app
        .list_pipeline_jobs(owner, &pipeline, 0, "")
        .await
        .unwrap()
        .jobs;
    assert_eq!(jobs.len(), 2);
    assert_eq!(var(&jobs[1], "SHA").as_deref(), Some("abc123"));
    assert_eq!(var(&jobs[1], "MODE").as_deref(), Some("push"));
}

/// The editors of the project manage the triggers. A viewer reads them, and cannot fire them.
#[sqlx::test(migrations = false)]
async fn the_trigger_routes_follow_the_roles(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (_, project, pipeline) = pipeline(&app).await;
    let (editor, viewer) = (app.user().await, app.user().await);
    let project_id = scylla_domain::projects::ProjectId::parse(&project).unwrap();
    app.grant(&[
        Project::EDITOR.of(project_id, editor),
        Project::VIEWER.of(project_id, viewer),
    ])
    .await;

    // when
    let created = app
        .create_trigger(editor, &pipeline, "nightly", Some("0 3 * * *"), Vec::new())
        .await;
    let id = text(&created.as_ref().unwrap().trigger.as_ref().unwrap().id).to_owned();
    let by_viewer = app
        .create_trigger(viewer, &pipeline, "other", Some("0 3 * * *"), Vec::new())
        .await;
    let read = app.get_trigger(viewer, &id).await;
    let fired = app.fire_trigger_now(viewer, &id).await;

    // then
    assert!(created.is_ok() && read.is_ok());
    assert_code(by_viewer, Code::PermissionDenied);
    assert_code(fired, Code::PermissionDenied);
}

/// A webhook trigger needs the key of the secrets.
#[sqlx::test(migrations = false)]
async fn a_webhook_needs_the_key_of_the_secrets(pool: PgPool) {
    // given
    let app = TestApp::spawn_without_secret_key(pool, scylla_ce::edition()).await;
    let (owner, _, pipeline) = pipeline(&app).await;

    // when
    let result = app
        .create_trigger(owner, &pipeline, "push", None, Vec::new())
        .await;

    // then
    let status = result.unwrap_err();
    assert_eq!(
        status.get_details_error_info().unwrap().reason,
        "SECRET_KEY_MISSING"
    );
}

/// More than 60 requests to one webhook in a minute give 429.
#[sqlx::test(migrations = false)]
async fn a_webhook_has_a_rate_limit(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _, pipeline) = pipeline(&app).await;
    let created = app
        .create_trigger(owner, &pipeline, "push", None, Vec::new())
        .await
        .unwrap();
    let Some(trigger::Kind::Webhook(webhook)) = created.trigger.unwrap().kind else {
        panic!("a webhook")
    };
    for _ in 0..60 {
        assert_eq!(post(&app, &webhook.path, "{}", "wrong", "x").await.0, 401);
    }

    // when
    let over = post(&app, &webhook.path, "{}", &created.webhook_secret, "y").await;

    // then
    assert_eq!(over.0, 429);
}
