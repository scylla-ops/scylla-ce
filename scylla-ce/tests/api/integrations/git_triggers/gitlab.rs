//! The tests of the runs of the GitLab merge requests: the report on the commit of the source
//! branch, and a merge request from a fork.

use scylla_proto::pipelines::v1::{JobStatus, SourceEvent};
use scylla_testkit::wait_for_requests;
use serde_json::json;
use sqlx::PgPool;

use super::{
    app_with_fake, deliver_gitlab, gitlab_merge_request, jobs_of, linked, pipeline_with, SHA,
};

/// A merge request of a branch of the project starts a run on the head commit of its source
/// branch, and the report goes to that commit with the source branch as its ref.
#[sqlx::test(migrations = false)]
async fn a_gitlab_merge_request_reports_on_the_commit_of_its_source_branch(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "gitlab").await;
    app.set_link_reports(&linked.project, true).await;
    let rules = [("pull_request", json!({ "branches": ["main"] }))];
    let (pipeline, _) = pipeline_with(&app, &linked, &rules).await;
    let statuses = format!("/api/v4/projects/7/statuses/{SHA}");
    fake.respond("POST", &statuses, 201, json!({ "id": 1 }), &[]);

    // when
    let opened = gitlab_merge_request("open", 17, SHA, (7, "group/app"));
    let done = deliver_gitlab(&app, &linked.project, "m-1", &opened).await;

    // then
    assert_eq!(done.error, "");
    let jobs = jobs_of(&app, &linked, &pipeline).await;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status(), JobStatus::Pending);
    let source = jobs[0].source.as_ref().unwrap();
    assert_eq!(source.event(), SourceEvent::PullRequest);
    assert_eq!(
        (source.sha.as_str(), source.branch.as_str()),
        (SHA, "feature")
    );
    let numbers: Vec<_> = source
        .pull_requests
        .iter()
        .map(|pull| pull.number)
        .collect();
    assert_eq!(numbers, vec![17]);
    let sent = wait_for_requests(&fake, "POST", &statuses, 1).await;
    assert_eq!(sent[0].body["ref"], "feature");
}

/// A merge request from a fork makes a run that waits for an approval, and the approval knows
/// that the merge request changes a pipeline file.
#[sqlx::test(migrations = false)]
async fn a_gitlab_merge_request_from_a_fork_waits_with_its_changed_files(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "gitlab").await;
    let (pipeline, _) = pipeline_with(&app, &linked, &[("pull_request", json!({}))]).await;
    fake.answer_merge_request_diffs(7, 18, &[".scylla/pipeline.toml"]);

    // when
    let opened = gitlab_merge_request("open", 18, SHA, (99, "evil/app"));
    let done = deliver_gitlab(&app, &linked.project, "m-1", &opened).await;

    // then
    assert_eq!(done.error, "");
    let jobs = jobs_of(&app, &linked, &pipeline).await;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status(), JobStatus::AwaitingApproval);
    assert!(jobs[0].approval.as_ref().unwrap().changes_pipeline_files);
    assert_eq!(jobs[0].source.as_ref().unwrap().fork_repository, "evil/app");
    let diffs = fake.requests_to("GET", "/api/v4/projects/7/merge_requests/18/diffs");
    assert_eq!(diffs.len(), 1);
    assert_eq!(diffs[0].header("private-token"), Some("glpat-runs"));
}
