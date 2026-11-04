//! The tests of the role `runner` on a project: an agent of some projects gets the jobs of these
//! projects only, and an agent of the workspace gets the others.

use std::time::Duration;

use scylla_proto::pipelines::v1::JobStatus;
use scylla_testkit::{exec_node, text, Role};
use sqlx::PgPool;

use super::runs::{agent, connect, pipeline};
use crate::app;

/// An agent with the role `runner` on one project, given with `Grant`, gets the job of this
/// project before an older job of another project, and the other job waits for an agent of the
/// workspace.
#[sqlx::test(migrations = false)]
async fn an_agent_of_one_project_gets_only_its_jobs(pool: PgPool) {
    // given
    let app = app(pool).await;
    let nodes = || vec![exec_node("build", &[])];
    let (owner, workspace, alpha, alpha_pipeline) = pipeline(&app, nodes()).await;
    let beta = app.create_project(owner, "beta").await.unwrap();
    let beta = text(&beta.id).to_owned();
    let beta_pipeline = app.create_pipeline(owner, &beta, nodes()).await.unwrap();
    let beta_job = app
        .run_pipeline(owner, text(&beta_pipeline.id))
        .await
        .unwrap();
    let beta_job = text(&beta_job.id).to_owned();
    let alpha_job = app.run_pipeline(owner, &alpha_pipeline).await.unwrap();
    let alpha_job = text(&alpha_job.id).to_owned();
    let (key, token) = app.app_in(owner, workspace, true).await;
    let runner = key.actor().to_string();
    app.grant_role(owner, Role::on("project", &alpha, "runner"), &runner)
        .await
        .unwrap();

    // when
    let mut scoped = connect(&app, key, &token, 4, "").await;
    let given = scoped.dispatch().await;
    let more = scoped.next(Duration::from_millis(500)).await;
    let waiting = app.get_job(owner, &beta_job).await.unwrap();
    let mut wide = agent(&app, owner, workspace, true, 4, "").await;
    let other = wide.dispatch().await;

    // then
    assert_eq!(text(&given.job_id), alpha_job);
    assert!(more.is_none(), "{more:?}");
    assert_eq!(waiting.status, JobStatus::Pending as i32);
    assert_eq!(text(&other.job_id), beta_job);
}
