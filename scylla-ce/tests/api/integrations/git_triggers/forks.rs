//! The tests of the runs of the pull requests from a fork: the run waits for an approval with the
//! files that the pull request changes, and a new commit replaces the run that waits.

use scylla_proto::pipelines::v1::{ApprovalState, JobStatus};
use serde_json::json;
use sqlx::PgPool;

use super::{
    app_with_fake, deliver_github, github_pull, jobs_of, linked, pipeline_with, OTHER_SHA, SHA,
};

/// A pull request from a fork makes a run that waits for an approval, and the approval knows
/// that the pull request changes a pipeline file. A new commit of the pull request cancels the
/// run of the old commit, and its new run waits.
#[sqlx::test(migrations = false)]
async fn a_pull_request_from_a_fork_waits_and_a_new_commit_replaces_its_run(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    let (pipeline, _) = pipeline_with(&app, &linked, &[("pull_request", json!({}))]).await;
    fake.answer_installation_token();
    let changed = [".scylla/pipelines/build.toml", "src/main.rs"];
    fake.answer_pull_files("octo/app", 57, &changed);
    fake.answer_pull_files("octo/app", 57, &["src/main.rs"]);
    let opened = github_pull("opened", 57, SHA, (99, "evil/app"), false);
    deliver_github(&app, "pull_request", "d-1", &opened).await;

    // when
    let pushed = github_pull("synchronize", 57, OTHER_SHA, (99, "evil/app"), false);
    let done = deliver_github(&app, "pull_request", "d-2", &pushed).await;

    // then
    assert_eq!(done.error, "");
    let jobs = jobs_of(&app, &linked, &pipeline).await;
    assert_eq!(jobs.len(), 2);
    let (new, old) = (&jobs[0], &jobs[1]);
    assert_eq!(new.status(), JobStatus::AwaitingApproval);
    let approval = new.approval.as_ref().unwrap();
    assert_eq!(approval.state(), ApprovalState::Waiting);
    assert!(!approval.changes_pipeline_files);
    assert_eq!(new.source.as_ref().unwrap().sha, OTHER_SHA);
    assert_eq!(old.status(), JobStatus::Cancelled);
    assert_eq!(old.error, "Replaced by the commit 89abcde");
    assert!(old.approval.as_ref().unwrap().changes_pipeline_files);
    let source = old.source.as_ref().unwrap();
    assert!(source.from_fork);
    assert_eq!(source.fork_repository, "evil/app");
    assert_eq!((source.sha.as_str(), source.branch.as_str()), (SHA, ""));
    let files = fake.requests_to("GET", "/repos/octo/app/pulls/57/files");
    assert_eq!(files.len(), 2);
    let tokens = fake.requests_to("POST", "/app/installations/42/access_tokens");
    let scope = json!({ "pull_requests": "read" });
    assert_eq!(tokens[0].body["permissions"], scope);
    assert_eq!(tokens[0].body["repository_ids"], json!([7]));
}

/// A pull request from a fork that the provider cannot list still waits, and its approval warns
/// that it can change a pipeline file.
#[sqlx::test(migrations = false)]
async fn a_fork_run_whose_files_cannot_be_read_warns_on_its_approval(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    let (pipeline, _) = pipeline_with(&app, &linked, &[("pull_request", json!({}))]).await;
    fake.answer_installation_token();

    // when
    let opened = github_pull("opened", 57, SHA, (99, "evil/app"), false);
    let done = deliver_github(&app, "pull_request", "d-1", &opened).await;

    // then
    assert_eq!(done.error, "");
    let jobs = jobs_of(&app, &linked, &pipeline).await;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status(), JobStatus::AwaitingApproval);
    assert!(jobs[0].approval.as_ref().unwrap().changes_pipeline_files);
}
