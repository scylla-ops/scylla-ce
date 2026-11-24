//! The tests of the jobs of a disabled agent: its running job ends as orphaned, and its job that
//! did not start goes back to the pool for another agent.

use std::time::Duration;

use scylla_proto::agents::v1::EventKind;
use scylla_proto::pipelines::v1::JobStatus;
use scylla_testkit::{exec_node, text};
use sqlx::PgPool;

use super::runs::{agent, pipeline};
use crate::app;

/// A disable ends the running job of the agent as orphaned, and gives its other job to another
/// agent.
#[sqlx::test(migrations = false)]
async fn disable_app_releases_the_jobs_of_its_agent(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, workspace, _, pipeline) = pipeline(&app, vec![exec_node("build", &[])]).await;
    let mut disabled = agent(&app, owner, workspace, true, 2, "").await;
    let first = text(&app.run_pipeline(owner, &pipeline).await.unwrap().id).to_owned();
    let second = text(&app.run_pipeline(owner, &pipeline).await.unwrap().id).to_owned();
    let started = disabled.dispatch().await;
    let waiting = disabled.dispatch().await;
    disabled
        .event(text(&started.job_id), EventKind::JobStarted, "")
        .await;
    let running = text(&started.job_id).to_owned();
    let pooled = text(&waiting.job_id).to_owned();
    for _ in 0..50 {
        let job = app.get_job(owner, &running).await.unwrap();
        if job.status == JobStatus::Running as i32 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // when
    app.disable_app(owner, &disabled.key.app).await.unwrap();
    let mut other = agent(&app, owner, workspace, true, 2, "").await;
    let given = other.dispatch().await;

    // then
    let mut dispatched = [running.clone(), pooled.clone()];
    dispatched.sort();
    let mut created = [first, second];
    created.sort();
    assert_eq!(dispatched, created);
    let orphaned = app.get_job(owner, &running).await.unwrap();
    assert_eq!(orphaned.status, JobStatus::Orphaned as i32);
    assert_eq!(text(&given.job_id), pooled);
}
