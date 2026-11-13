//! The tests of the re-runs that a person asks for on GitHub: `check_run` and `check_suite` with
//! the action `rerequested` start a new run of the job of each check run that ended.

use scylla_proto::pipelines::v1::SourceEvent;
use scylla_testkit::{text, wait_for_requests, FakeProvider};
use sqlx::PgPool;

use super::{app_with, linked, post_github, rerequested, run_on_main, CHECK_RUNS, SHA};

/// Re-run on a check run starts a new run of its job, with the job as `rerun_of` and the event
/// of a re-run on the provider, and the new run makes a check run of the same name on the same
/// commit.
#[sqlx::test(migrations = false)]
async fn a_rerequested_check_run_runs_its_job_again(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let app = app_with(pool, &fake).await;
    let linked = linked(&app).await;
    let job = run_on_main(&app, &linked).await;
    app.cancel_job(linked.owner, &job).await.unwrap();
    app.wait_for_sent_state(linked.owner, &job, "cancelled")
        .await;

    // when
    let body = rerequested("check_run", SHA, Some(&job));
    let status = post_github(&app, "check_run", "d-rerun", &body).await;
    let created = wait_for_requests(&fake, "POST", CHECK_RUNS, 2).await;

    // then
    assert_eq!(status, 202);
    let jobs = app
        .list_pipeline_jobs(linked.owner, &linked.pipeline, 10, "")
        .await
        .unwrap()
        .jobs;
    assert_eq!(jobs.len(), 2);
    let new = &jobs[0];
    assert_eq!(text(&new.rerun_of), job);
    let source = new.source.as_ref().expect("a source");
    assert_eq!(source.event(), SourceEvent::ProviderRerun);
    assert_eq!(source.sha, SHA);
    let again = &created[1].body;
    assert_eq!(again["external_id"], text(&new.id));
    assert_eq!(again["name"], "scylla/build");
    assert_eq!(again["head_sha"], SHA);
}

/// Re-run of a whole suite starts a new run of the newest job of each check run of the commit.
#[sqlx::test(migrations = false)]
async fn a_rerequested_suite_runs_the_newest_job_of_each_check(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let app = app_with(pool, &fake).await;
    let linked = linked(&app).await;
    let first = run_on_main(&app, &linked).await;
    app.cancel_job(linked.owner, &first).await.unwrap();
    let second = app.rerun_job(linked.owner, &first).await.unwrap();
    let second = text(&second.id).to_owned();
    app.cancel_job(linked.owner, &second).await.unwrap();
    app.wait_for_sent_state(linked.owner, &second, "cancelled")
        .await;

    // when
    let body = rerequested("check_suite", SHA, None);
    let status = post_github(&app, "check_suite", "d-suite", &body).await;

    // then
    assert_eq!(status, 202);
    let jobs = app
        .list_pipeline_jobs(linked.owner, &linked.pipeline, 10, "")
        .await
        .unwrap()
        .jobs;
    assert_eq!(jobs.len(), 3);
    assert_eq!(text(&jobs[0].rerun_of), second);
}

/// A request for a job that did not end, for an id that is not a job, or for another commit
/// starts no run, and the delivery ends with no error.
#[sqlx::test(migrations = false)]
async fn a_rerun_request_skips_a_check_that_cannot_run_again(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let app = app_with(pool, &fake).await;
    let linked = linked(&app).await;
    let open = run_on_main(&app, &linked).await;
    app.wait_for_sent_state(linked.owner, &open, "queued").await;
    let other = "89abcdef0123456789abcdef0123456789abcdef";

    // when
    let not_ended = rerequested("check_run", SHA, Some(&open));
    let not_a_job = rerequested("check_run", SHA, Some("not-a-job"));
    let other_commit = rerequested("check_suite", other, None);
    post_github(&app, "check_run", "d-open", &not_ended).await;
    post_github(&app, "check_run", "d-bad", &not_a_job).await;
    post_github(&app, "check_suite", "d-other", &other_commit).await;

    // then
    let jobs = app
        .list_pipeline_jobs(linked.owner, &linked.pipeline, 10, "")
        .await
        .unwrap()
        .jobs;
    assert_eq!(jobs.len(), 1);
    let deliveries = app.deliveries().await;
    assert_eq!(deliveries.len(), 3);
    assert!(
        deliveries.iter().all(|row| row.error.is_empty()),
        "{deliveries:?}"
    );
}
