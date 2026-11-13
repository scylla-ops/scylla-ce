//! The tests of the access to the reports: the readers of a project list them, its editors send
//! one again, and the limits of a request.

use scylla_proto::integrations::v1::ReportState;
use scylla_testkit::{FakeProvider, Role};
use sqlx::PgPool;
use tonic::Code;

use super::{app_with, github, linked, run_on_main};
use crate::assert_code;

/// An unknown job id.
const UNKNOWN: &str = "0190a0a0-0000-7000-8000-000000000000";

/// A viewer of the workspace lists the reports of its jobs and cannot send one again. A member
/// sends one again. A stranger reads nothing, and an unknown job is not in the list.
#[sqlx::test(migrations = false)]
async fn only_readers_list_the_reports_and_only_editors_retry(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github(&fake, 422);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github").await;
    let workspace = linked.workspace.to_string();
    let (viewer, member, stranger) = (app.user().await, app.user().await, app.user().await);
    for (actor, role) in [(viewer, "viewer"), (member, "member")] {
        let grant = Role::on("workspace", &workspace, role);
        app.grant_role(linked.owner, grant, &actor.to_string())
            .await
            .unwrap();
    }
    let job = run_on_main(&app, &linked).await;
    app.wait_for_report(linked.owner, &job, ReportState::NotSent)
        .await;

    // when
    let viewer_list = app.list_job_reports(viewer, &[&job, UNKNOWN]).await;
    let stranger_list = app.list_job_reports(stranger, &[&job]).await;
    let viewer_retry = app.retry_job_report(viewer, &job, "scylla/build").await;
    let member_retry = app.retry_job_report(member, &job, "scylla/build").await;
    let stranger_retry = app.retry_job_report(stranger, &job, "scylla/build").await;

    // then
    let jobs = viewer_list.unwrap().jobs;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].reports[0].context, "scylla/build");
    assert!(jobs[0].no_report_reason.is_empty());
    assert_code(stranger_list, Code::PermissionDenied);
    assert_code(viewer_retry, Code::PermissionDenied);
    assert_eq!(member_retry.unwrap().state(), ReportState::Sending);
    assert_code(stranger_retry, Code::PermissionDenied);
}

/// A list takes 1 to 100 jobs. A list of unknown jobs and a retry of an unknown job are
/// refusals, and an unknown name of a report is not found.
#[sqlx::test(migrations = false)]
async fn a_request_of_the_reports_has_limits(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github(&fake, 201);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github").await;
    let job = run_on_main(&app, &linked).await;
    let many = vec![job.as_str(); 101];

    // when
    let empty = app.list_job_reports(linked.owner, &[]).await;
    let too_many = app.list_job_reports(linked.owner, &many).await;
    let unknown = app.list_job_reports(linked.owner, &[UNKNOWN]).await;
    let unknown_job = app
        .retry_job_report(linked.owner, UNKNOWN, "scylla/build")
        .await;
    let unknown_name = app
        .retry_job_report(linked.owner, &job, "scylla/other")
        .await;
    let no_name = app.retry_job_report(linked.owner, &job, " ").await;

    // then
    assert_code(empty, Code::InvalidArgument);
    assert_code(too_many, Code::InvalidArgument);
    assert_code(unknown, Code::PermissionDenied);
    assert_code(unknown_job, Code::PermissionDenied);
    assert_code(unknown_name, Code::NotFound);
    assert_code(no_name, Code::InvalidArgument);
}
