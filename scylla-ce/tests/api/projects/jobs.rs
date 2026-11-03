//! The tests of the jobs that people call: the cancel, the rerun, the delete and the lists.

use scylla_proto::pipelines::v1::JobStatus;
use scylla_testkit::{exec_node, text, TestApp};
use sqlx::PgPool;
use tonic::Code;
use tonic_types::StatusExt;

use crate::{app, assert_code};

/// Creates a user, a project and a pipeline of one node. Returns the user and the pipeline id.
async fn pipeline(app: &TestApp) -> (scylla_domain::identity::actors::ActorId, String) {
    let owner = app.user().await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let pipeline = app
        .create_pipeline(owner, text(&project.id), vec![exec_node("build", &[])])
        .await
        .unwrap();
    (owner, text(&pipeline.id).to_owned())
}

/// A cancel ends a pending job and its nodes. A second cancel changes nothing. A rerun makes a
/// new pending job of a job that ended, and a job that did not end gives `JOB_NOT_ENDED`.
#[sqlx::test(migrations = false)]
async fn cancel_and_rerun_follow_the_state(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, pipeline) = pipeline(&app).await;
    let job = app.run_pipeline(owner, &pipeline).await.unwrap();
    let id = text(&job.id).to_owned();
    let early = app.rerun_job(owner, &id).await;

    // when
    let cancelled = app.cancel_job(owner, &id).await.unwrap();
    let again = app.cancel_job(owner, &id).await.unwrap();
    let rerun = app.rerun_job(owner, &id).await.unwrap();

    // then
    let status = early.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert_eq!(
        status.get_details_error_info().unwrap().reason,
        "JOB_NOT_ENDED"
    );
    assert_eq!(cancelled.status, JobStatus::Cancelled as i32);
    assert_eq!(
        cancelled.runs[0].status,
        scylla_proto::pipelines::v1::NodeStatus::Cancelled as i32
    );
    assert_eq!(again.finished_at, cancelled.finished_at);
    assert_eq!(rerun.status, JobStatus::Pending as i32);
    assert_eq!(text(&rerun.rerun_of), id);
}

/// The lists give the newest job first, with pages. A deleted job is gone.
#[sqlx::test(migrations = false)]
async fn list_jobs_gives_the_newest_first(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, pipeline) = pipeline(&app).await;
    let mut ids = Vec::new();
    for _ in 0..3 {
        ids.push(text(&app.run_pipeline(owner, &pipeline).await.unwrap().id).to_owned());
    }
    let workspace = app.workspace_of(owner).await;

    // when
    let first = app
        .list_pipeline_jobs(owner, &pipeline, 2, "")
        .await
        .unwrap();
    let token = first.page.clone().unwrap().next_token;
    let second = app
        .list_pipeline_jobs(owner, &pipeline, 2, &token)
        .await
        .unwrap();
    app.delete_job(owner, &ids[0]).await.unwrap();
    let in_workspace = app
        .list_workspace_jobs(owner, workspace, None)
        .await
        .unwrap();

    // then
    let listed: Vec<_> = first
        .jobs
        .iter()
        .chain(&second.jobs)
        .map(|job| text(&job.id))
        .collect();
    assert_eq!(
        listed,
        vec![ids[2].as_str(), ids[1].as_str(), ids[0].as_str()]
    );
    assert_eq!(in_workspace.jobs.len(), 2);
    assert_code(app.get_job(owner, &ids[0]).await, Code::PermissionDenied);
}

/// Another user cannot read, cancel or list the jobs.
#[sqlx::test(migrations = false)]
async fn the_job_routes_need_a_role(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, pipeline) = pipeline(&app).await;
    let stranger = app.user().await;
    let id = text(&app.run_pipeline(owner, &pipeline).await.unwrap().id).to_owned();
    let workspace = app.workspace_of(owner).await;

    // when
    let read = app.get_job(stranger, &id).await;
    let cancel = app.cancel_job(stranger, &id).await;
    let listed = app.list_pipeline_jobs(stranger, &pipeline, 0, "").await;
    let in_workspace = app.list_workspace_jobs(stranger, workspace, None).await;
    let tail = app.tail_job_logs(stranger, &id, "", "").await;

    // then
    assert_code(read, Code::PermissionDenied);
    assert_code(cancel, Code::PermissionDenied);
    assert_code(listed, Code::PermissionDenied);
    assert_code(in_workspace, Code::PermissionDenied);
    assert_code(tail, Code::PermissionDenied);
}
