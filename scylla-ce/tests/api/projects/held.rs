//! The tests of the number of jobs that an agent holds, on its App.

use std::time::Duration;

use scylla_domain::identity::actors::ActorId;
use scylla_proto::agents::v1::EventKind;
use scylla_testkit::{exec_node, text, TestApp};
use sqlx::PgPool;

use super::runs::{agent, pipeline};
use crate::app;

/// Reads the number of jobs of an App until it is `count`, for at most 5 seconds, and returns
/// the last number.
async fn running_jobs(app: &TestApp, owner: ActorId, id: &str, count: u32) -> u32 {
    let mut read = 0;
    for _ in 0..50 {
        let response = app.get_app(owner, id).await.unwrap();
        read = response.app.unwrap().running_jobs;
        if read == count {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    read
}

/// An App counts the jobs that its agent took and that did not end: a started job and a job
/// that waits. A job that ends leaves the number.
#[sqlx::test(migrations = false)]
async fn an_app_counts_the_jobs_that_its_agent_holds(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, workspace, _, pipeline) = pipeline(&app, vec![exec_node("build", &[])]).await;
    let mut busy = agent(&app, owner, workspace, true, 2, "").await;
    app.run_pipeline(owner, &pipeline).await.unwrap();
    app.run_pipeline(owner, &pipeline).await.unwrap();
    let started = busy.dispatch().await;
    busy.dispatch().await;
    let started = text(&started.job_id).to_owned();
    busy.event(&started, EventKind::JobStarted, "").await;
    let idle = agent(&app, owner, workspace, true, 2, "").await;

    // when
    let both = running_jobs(&app, owner, &busy.key.app, 2).await;
    busy.event(&started, EventKind::JobFailed, "").await;
    let one = running_jobs(&app, owner, &busy.key.app, 1).await;

    // then
    assert_eq!((both, one), (2, 1));
    let listed = app.list_apps(owner, workspace, Some(true), 10, "").await;
    let count = |id: &str| {
        let apps = &listed.as_ref().unwrap().apps;
        let found = apps.iter().find(|listed| text(&listed.id) == id);
        found.unwrap().running_jobs
    };
    assert_eq!((count(&busy.key.app), count(&idle.key.app)), (1, 0));
}
