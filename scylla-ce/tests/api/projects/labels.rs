//! The tests of the labels of the jobs: a pipeline asks labels, a job keeps them, and only an
//! agent with each of them gets the job.

use std::time::Duration;

use scylla_proto::agents::v1::open_stream_response::Message as Down;
use scylla_testkit::{exec_node, text};
use sqlx::PgPool;

use super::runs::{agent, pipeline};
use crate::{app, assert_invalid_field};

/// A job with labels goes only to an agent with each of them, from its host or from the admins.
/// While no such agent is online, the job tells why it waits. A new label of the admins gives
/// the job to the agent at once.
#[sqlx::test(migrations = false)]
async fn a_job_with_labels_goes_only_to_an_agent_with_the_labels(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, workspace, _, pipeline) = pipeline(&app, vec![exec_node("deploy", &[])]).await;
    let version = app.get_pipeline(owner, &pipeline).await.unwrap().version;
    let labels = ["deploy", "linux"];
    app.set_pipeline_labels(owner, &pipeline, version, &labels)
        .await
        .unwrap();
    let mut ci = agent(&app, owner, workspace, true, 2, "").await;
    let mut prod = agent(&app, owner, workspace, true, 2, "").await;

    // when
    let job = text(&app.run_pipeline(owner, &pipeline).await.unwrap().id).to_owned();
    let waiting = app.get_job(owner, &job).await.unwrap();
    let to_ci = ci.next(Duration::from_millis(500)).await;
    let prod_app = prod.key.app.clone();
    app.set_app_labels(owner, &prod_app, &["deploy"])
        .await
        .unwrap();
    let dispatch = prod.dispatch().await;

    // then
    assert_eq!(waiting.agent_labels, labels);
    assert_eq!(waiting.waiting_reason, "NO_AGENT_WITH_LABELS");
    assert!(!matches!(to_ci, Some(Down::Dispatch(_))), "{to_ci:?}");
    assert_eq!(text(&dispatch.job_id), job);
    let placed = app.get_job(owner, &job).await.unwrap();
    assert_eq!(text(&placed.agent_id), prod_app);
}

/// A job with no label goes to any agent, also to an agent with labels.
#[sqlx::test(migrations = false)]
async fn a_job_with_no_labels_goes_to_any_agent(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, workspace, _, pipeline) = pipeline(&app, vec![exec_node("build", &[])]).await;
    let mut labelled = agent(&app, owner, workspace, true, 1, "").await;
    let labelled_app = labelled.key.app.clone();
    app.set_app_labels(owner, &labelled_app, &["deploy"])
        .await
        .unwrap();

    // when
    let job = app.run_pipeline(owner, &pipeline).await.unwrap();
    let dispatch = labelled.dispatch().await;

    // then
    assert!(job.agent_labels.is_empty());
    assert_eq!(dispatch.job_id, job.id);
}

/// A new run keeps the labels of its job, also after a change of the labels of the pipeline.
/// The pipeline shows its labels, and an empty list removes them.
#[sqlx::test(migrations = false)]
async fn rerun_job_keeps_the_labels_of_its_job(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _, _, pipeline) = pipeline(&app, vec![exec_node("deploy", &[])]).await;
    let version = app.get_pipeline(owner, &pipeline).await.unwrap().version;
    let set = app.set_pipeline_labels(owner, &pipeline, version, &["deploy"]);
    let set = set.await.unwrap();
    let first = text(&app.run_pipeline(owner, &pipeline).await.unwrap().id).to_owned();
    app.cancel_job(owner, &first).await.unwrap();
    let cleared = app.set_pipeline_labels(owner, &pipeline, set.version, &[]);
    let cleared = cleared.await.unwrap();

    // when
    let rerun = app.rerun_job(owner, &first).await.unwrap();
    let fresh = app.run_pipeline(owner, &pipeline).await.unwrap();

    // then
    assert_eq!(set.agent.unwrap().labels, ["deploy"]);
    assert!(cleared.agent.unwrap().labels.is_empty());
    assert_eq!(rerun.agent_labels, ["deploy"]);
    assert!(fresh.agent_labels.is_empty());
}

/// A pipeline asks labels of the form of a label, at most 16, and the refusal names the bad
/// value.
#[sqlx::test(migrations = false)]
async fn update_pipeline_checks_the_labels(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _, _, pipeline) = pipeline(&app, vec![exec_node("build", &[])]).await;
    let version = app.get_pipeline(owner, &pipeline).await.unwrap().version;
    let many: Vec<String> = (0..17).map(|n| format!("l{n}")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();

    // when
    let bad = app.set_pipeline_labels(owner, &pipeline, version, &["ok", "Not ok"]);
    let bad = bad.await;
    let too_many = app.set_pipeline_labels(owner, &pipeline, version, &many);
    let too_many = too_many.await;

    // then
    assert_invalid_field(&bad.unwrap_err(), "agent.labels[1]");
    assert_invalid_field(&too_many.unwrap_err(), "agent.labels");
}
