//! The tests of the approval of the runs from a fork: who approves or rejects a job that waits,
//! the state of the job after a decision, its secrets, and its report on the commit.

use std::collections::HashMap;
use std::time::Duration;

use scylla_core::api::ServeOptions;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::pipelines::v1::job_service_client::JobServiceClient;
use scylla_proto::pipelines::v1::{
    env_var, ApprovalState, ApproveJobRequest, EnvVar, JobStatus, Node,
};
use scylla_testkit::{exec_node, id, text, wait_for_requests, FakeProvider, LinkRow, TestApp};
use serde_json::json;
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

use crate::assert_code;

/// A commit: 40 hexadecimal characters.
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

/// Another commit.
const OTHER_SHA: &str = "89abcdef0123456789abcdef0123456789abcdef";

/// A linked project of a test.
struct Linked {
    /// The owner of the workspace.
    owner: ActorId,
    /// The workspace.
    workspace: WorkspaceId,
    /// The project.
    project: String,
    /// The pipeline, named `build`.
    pipeline: String,
}

/// Starts a server that calls a fake GitHub with the test GitHub App: the installation 42 gets a
/// token, and the commit statuses of both commits are created.
async fn app_with(pool: PgPool, fake: &FakeProvider) -> TestApp {
    let token = json!({ "token": "ghs_approvals", "expires_at": "2099-01-01T00:00:00Z" });
    let path = "/app/installations/42/access_tokens";
    fake.respond("POST", path, 201, token, &[]);
    for sha in [SHA, OTHER_SHA] {
        let answer = json!({ "id": 1, "state": "pending" });
        fake.respond("POST", &statuses(sha), 201, answer, &[]);
    }
    let options = ServeOptions {
        vcs: fake.options(true),
        ..ServeOptions::default()
    };
    TestApp::spawn_with(pool, scylla_ce::edition(), options).await
}

/// Returns the path of the commit statuses of a commit of `octo/app`.
fn statuses(sha: &str) -> String {
    format!("/repos/octo/app/statuses/{sha}")
}

/// Makes a project with one pipeline of these nodes, linked to `octo/app` (id 7) through the
/// installation 42.
async fn linked(app: &TestApp, nodes: Vec<Node>) -> Linked {
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let project = text(&project.id).to_owned();
    let pipeline = app.create_pipeline(owner, &project, nodes).await.unwrap();
    let pipeline = text(&pipeline.id).to_owned();
    let connection = app
        .insert_github_connection(workspace, owner, 42, "octo")
        .await;
    let link = LinkRow {
        project: &project,
        connection: &connection,
        external_id: "7",
        full_name: "octo/app",
        default_branch: "main",
    };
    app.insert_repository_link(&link).await;
    Linked {
        owner,
        workspace,
        project,
        pipeline,
    }
}

/// Runs the pipeline of a linked project, makes the job a job from a fork on a commit that waits
/// for an approval, and returns it.
async fn waiting(app: &TestApp, linked: &Linked, sha: &str) -> String {
    let job = app
        .run_pipeline(linked.owner, &linked.pipeline)
        .await
        .unwrap();
    let job = text(&job.id).to_owned();
    app.make_fork_job_wait(&job, sha).await;
    job
}

/// Checks that a refused call is `FAILED_PRECONDITION` with this reason.
fn assert_reason<T: std::fmt::Debug>(result: Result<T, Status>, reason: &str) {
    let status = result.expect_err("the call must fail");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    let info = status.get_details_error_info().expect("ErrorInfo");
    assert_eq!(info.reason, reason);
}

/// An approval makes the job pending with the editor and the time, and an agent that waits gets
/// it. The job runs with an empty value for each secret, and `SCYLLA_GIT_FORK` is true.
#[sqlx::test(migrations = false)]
async fn approve_job_runs_the_job_with_no_secret_value(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let app = app_with(pool, &fake).await;
    let mut node = exec_node("build", &[]);
    let var = |name: &str, value| EnvVar {
        name: name.to_owned(),
        value: Some(value),
    };
    node.env = vec![
        var("TOKEN", env_var::Value::Secret("API_TOKEN".to_owned())),
        var("MODE", env_var::Value::Literal("ci".to_owned())),
    ];
    let linked = linked(&app, vec![node]).await;
    let secret = app.create_secret(linked.owner, &linked.project, "API_TOKEN", "s3cret");
    secret.await.unwrap();
    let job = waiting(&app, &linked, SHA).await;

    // when
    let approve = async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        app.approve_job(linked.owner, &job).await
    };
    let ((dispatch, _up), approved) =
        tokio::join!(app.first_dispatch(linked.owner, linked.workspace), approve);

    // then
    let approved = approved.unwrap();
    assert_eq!(approved.status, JobStatus::Pending as i32);
    let approval = approved.approval.expect("an approval");
    assert_eq!(approval.state, ApprovalState::Approved as i32);
    assert_eq!(text(&approval.decided_by), linked.owner.to_string());
    assert!(approval.decided_at.is_some() && approved.error.is_empty());
    assert_eq!(text(&dispatch.job_id), job);
    let env: HashMap<_, _> = dispatch.nodes[0]
        .env
        .iter()
        .map(|var| (var.name.as_str(), (var.value.as_str(), var.masked)))
        .collect();
    assert_eq!(env["TOKEN"], ("", false));
    assert_eq!(env["MODE"], ("ci", false));
    assert_eq!(env["SCYLLA_GIT_FORK"], ("true", false));
    assert_eq!(env["SCYLLA_PR_NUMBER"], ("57", false));
}

/// Only an actor with the edit of the project approves or rejects a job from a fork. A call with
/// no token, a viewer of the project and a person with no role get a refusal, and the job still
/// waits.
#[sqlx::test(migrations = false)]
async fn approve_job_and_reject_job_need_the_edit_of_the_project(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, vec![exec_node("build", &[])]).await;
    let (editor, viewer, outsider) = (app.user().await, app.user().await, app.user().await);
    let project = ProjectId::parse(&linked.project).unwrap();
    app.grant(&[
        Project::EDITOR.of(project, editor),
        Project::VIEWER.of(project, viewer),
    ])
    .await;
    let first = waiting(&app, &linked, SHA).await;
    let second = waiting(&app, &linked, OTHER_SHA).await;

    // when
    let anonymous = JobServiceClient::new(app.channel.clone())
        .approve_job(ApproveJobRequest {
            job_id: Some(id(&first)),
        })
        .await;
    let approved_by_viewer = app.approve_job(viewer, &first).await;
    let rejected_by_viewer = app.reject_job(viewer, &first).await;
    let approved_by_outsider = app.approve_job(outsider, &first).await;
    let rejected_by_outsider = app.reject_job(outsider, &first).await;
    let read = app.get_job(viewer, &first).await.unwrap();
    let approved = app.approve_job(editor, &first).await;
    let rejected = app.reject_job(editor, &second).await;

    // then
    assert_code(anonymous, Code::Unauthenticated);
    assert_code(approved_by_viewer, Code::PermissionDenied);
    assert_code(rejected_by_viewer, Code::PermissionDenied);
    assert_code(approved_by_outsider, Code::PermissionDenied);
    assert_code(rejected_by_outsider, Code::PermissionDenied);
    assert_eq!(read.status, JobStatus::AwaitingApproval as i32);
    let waits = read.approval.map(|approval| approval.state);
    assert_eq!(waits, Some(ApprovalState::Waiting as i32));
    assert_eq!(approved.unwrap().status, JobStatus::Pending as i32);
    assert_eq!(rejected.unwrap().status, JobStatus::Cancelled as i32);
}

/// A rejection cancels the job with the name of the editor, and no later decision changes it.
#[sqlx::test(migrations = false)]
async fn reject_job_cancels_the_job_with_the_name_of_the_editor(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, vec![exec_node("build", &[])]).await;
    let job = waiting(&app, &linked, SHA).await;

    // when
    let rejected = app.reject_job(linked.owner, &job).await.unwrap();
    let approved = app.approve_job(linked.owner, &job).await;
    let read = app.get_job(linked.owner, &job).await.unwrap();

    // then
    assert_eq!(rejected.status, JobStatus::Cancelled as i32);
    assert_eq!(rejected.error, "Rejected by Test User");
    assert!(rejected.finished_at.is_some());
    let approval = rejected.approval.expect("an approval");
    assert_eq!(approval.state, ApprovalState::Rejected as i32);
    assert_eq!(text(&approval.decided_by), linked.owner.to_string());
    assert!(approval.decided_at.is_some());
    assert_reason(approved, "JOB_NOT_AWAITING_APPROVAL");
    assert_eq!(read.status, JobStatus::Cancelled as i32);
}

/// A job that does not wait for an approval refuses a decision with
/// `JOB_NOT_AWAITING_APPROVAL`, and does not change.
#[sqlx::test(migrations = false)]
async fn approve_job_refuses_a_job_that_does_not_wait(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, vec![exec_node("build", &[])]).await;
    let owner = linked.owner;
    let pending = app.run_pipeline(owner, &linked.pipeline).await.unwrap();
    let pending = text(&pending.id).to_owned();
    let approved = waiting(&app, &linked, SHA).await;
    app.approve_job(owner, &approved).await.unwrap();

    // when
    let refusals = [
        app.approve_job(owner, &pending).await,
        app.reject_job(owner, &pending).await,
        app.approve_job(owner, &approved).await,
        app.reject_job(owner, &approved).await,
    ];
    let pending = app.get_job(owner, &pending).await.unwrap();
    let approved = app.get_job(owner, &approved).await.unwrap();

    // then
    for refusal in refusals {
        assert_reason(refusal, "JOB_NOT_AWAITING_APPROVAL");
    }
    assert_eq!(pending.status, JobStatus::Pending as i32);
    assert!(pending.approval.is_none());
    assert_eq!(approved.status, JobStatus::Pending as i32);
    let state = approved.approval.map(|approval| approval.state);
    assert_eq!(state, Some(ApprovalState::Approved as i32));
}

/// The report of a job on its commit follows the decision: `Queued` after an approval, and
/// `Cancelled` after a rejection.
#[sqlx::test(migrations = false)]
async fn a_decision_changes_the_report_of_the_job(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, vec![exec_node("build", &[])]).await;
    let approved = waiting(&app, &linked, SHA).await;
    let rejected = waiting(&app, &linked, OTHER_SHA).await;

    // when
    app.approve_job(linked.owner, &approved).await.unwrap();
    app.reject_job(linked.owner, &rejected).await.unwrap();
    let queued = wait_for_requests(&fake, "POST", &statuses(SHA), 1).await;
    let cancelled = wait_for_requests(&fake, "POST", &statuses(OTHER_SHA), 1).await;

    // then
    let queued = &queued[0].body;
    assert_eq!(queued["state"], "pending");
    assert_eq!(queued["context"], "scylla/build");
    assert_eq!(queued["description"], "Queued");
    let cancelled = &cancelled[0].body;
    assert_eq!(cancelled["state"], "error");
    assert_eq!(cancelled["context"], "scylla/build");
    assert_eq!(cancelled["description"], "Cancelled");
}
