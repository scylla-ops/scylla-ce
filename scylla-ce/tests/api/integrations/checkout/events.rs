//! The tests of the events and of the log lines of a checkout on the stream of an agent.

use std::time::Duration;

use scylla_proto::agents::v1::EventKind;
use scylla_proto::pipelines::v1::git_ref::Kind;
use scylla_proto::pipelines::v1::{JobStatus, NodeStatus};
use scylla_testkit::{text, FakeProvider};
use sqlx::PgPool;

use super::{app_with, github, job_when, linked};

/// The events of the agent start and complete the checkout of a running job, with its times.
#[sqlx::test(migrations = false)]
async fn checkout_events_change_the_checkout_of_the_job(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github(&fake);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github", None).await;
    let main = Kind::Branch("main".to_owned());
    let job = app.run_pipeline_on(linked.owner, &linked.pipeline, main);
    let job = text(&job.await.unwrap().id).to_owned();
    let mut agent = app
        .agent_with(linked.owner, linked.workspace, &["checkout"])
        .await;
    agent.dispatch_in(Duration::from_secs(10)).await.unwrap();

    // when
    agent.event(&job, EventKind::JobStarted, "").await;
    agent.event(&job, EventKind::CheckoutStarted, "").await;
    let started = job_when(&app, linked.owner, &job, |job| {
        job.checkout.as_ref().unwrap().status == NodeStatus::Running as i32
    })
    .await;
    agent.event(&job, EventKind::CheckoutCompleted, "").await;
    let completed = job_when(&app, linked.owner, &job, |job| {
        job.checkout.as_ref().unwrap().status == NodeStatus::Completed as i32
    })
    .await;

    // then
    assert_eq!(started.status, JobStatus::Running as i32);
    let run = completed.checkout.unwrap();
    assert!(run.started_at.is_some() && run.finished_at.is_some());
    assert_eq!(run.error, "");
    assert_eq!(completed.status, JobStatus::Running as i32);
}

/// A failed checkout keeps its error, its log lines on the node `.checkout` reach the tail, and
/// the job fails.
#[sqlx::test(migrations = false)]
async fn a_failed_checkout_keeps_its_error_and_its_lines(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github(&fake);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github", None).await;
    let main = Kind::Branch("main".to_owned());
    let job = app.run_pipeline_on(linked.owner, &linked.pipeline, main);
    let job = text(&job.await.unwrap().id).to_owned();
    let mut agent = app
        .agent_with(linked.owner, linked.workspace, &["checkout"])
        .await;
    agent.dispatch_in(Duration::from_secs(10)).await.unwrap();

    // when
    agent.event(&job, EventKind::JobStarted, "").await;
    agent.event(&job, EventKind::CheckoutStarted, "").await;
    agent.log(&job, ".checkout", "git fetch --no-tags").await;
    agent
        .event(&job, EventKind::CheckoutFailed, "no such commit")
        .await;
    agent
        .event(
            &job,
            EventKind::JobFailed,
            "the checkout failed: no such commit",
        )
        .await;
    let failed = job_when(&app, linked.owner, &job, |job| {
        job.status == JobStatus::Failed as i32
    })
    .await;
    let tail = app.tail_job_logs(linked.owner, &job, ".checkout", "").await;

    // then
    let run = failed.checkout.unwrap();
    assert_eq!(run.status, NodeStatus::Failed as i32);
    assert_eq!(run.error, "no such commit");
    assert_eq!(failed.error, "the checkout failed: no such commit");
    let lines: Vec<String> = tail
        .unwrap()
        .into_iter()
        .flat_map(|batch| batch.lines)
        .map(|line| line.text)
        .collect();
    assert_eq!(lines, ["git fetch --no-tags"]);
}
