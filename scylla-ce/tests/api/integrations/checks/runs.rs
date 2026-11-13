//! The tests of the check run of a run: one check run for each pipeline, made one time and then
//! changed, with its output, and the commit statuses of a link in the commit status mode.

use scylla_proto::integrations::v1::ReportMode;
use scylla_testkit::{wait_for_requests, FakeProvider, Logs};
use serde_json::json;
use sqlx::PgPool;

use super::{app_with, linked, run_on_main, CHECK_PAGE, CHECK_RUN, CHECK_RUNS, GITHUB_TOKEN, SHA};

/// A run makes its check run one time, with the name of its report, its commit, its job and the
/// link of the run, and the token is narrowed to the repository and to the check runs. The end
/// changes the check run: completed with the conclusion of the map, a title with the nodes that
/// passed, a table of the nodes, and the last 50 log lines of the failed node.
#[sqlx::test(migrations = false)]
async fn a_failed_run_on_github_completes_its_check_run(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let app = app_with(pool, &fake).await;
    let linked = linked(&app).await;
    let mut agent = app.run_agent(linked.owner, linked.workspace).await;
    let lines: Vec<String> = (0..60).map(|n| format!("line {n:02}")).collect();
    let logs = Logs::json();

    // when
    let job = run_on_main(&app, &linked).await;
    let created = wait_for_requests(&fake, "POST", CHECK_RUNS, 1).await;
    let ran = agent.fail_next_job(&["build"], "test", &lines).await;
    let report = app.wait_for_sent_state(linked.owner, &job, "failure").await;

    // then
    assert_eq!(ran, job);
    let created = &created[0];
    let page = format!(
        "/projects/{}/pipelines/{}/jobs/{job}",
        linked.project, linked.pipeline
    );
    assert_eq!(created.body["name"], "scylla/build");
    assert_eq!(created.body["head_sha"], SHA);
    assert_eq!(created.body["external_id"], job.as_str());
    assert_eq!(created.body["status"], "queued");
    assert_eq!(created.body["output"]["title"], "Queued");
    assert!(created.body["details_url"]
        .as_str()
        .unwrap()
        .ends_with(&page));
    assert_eq!(created.header("authorization"), Some("Bearer ghs_checks"));
    let changes = fake.requests_to("PATCH", CHECK_RUN);
    let last = &changes.last().expect("a change of the check run").body;
    assert_eq!(last["status"], "completed");
    assert_eq!(last["conclusion"], "failure");
    assert!(last.get("head_sha").is_none());
    let output = &last["output"];
    assert_eq!(output["title"], "build failed: 1/2 nodes passed");
    let summary = output["summary"].as_str().unwrap();
    assert!(summary.contains(&page), "{summary}");
    assert!(summary.contains("| `build` | ✓ Passed |"), "{summary}");
    assert!(summary.contains("| `test` | ✗ Failed |"), "{summary}");
    let text = output["text"].as_str().unwrap();
    assert!(text.starts_with("Last 50 lines of `test`:"), "{text}");
    assert!(
        text.contains("line 10\n") && text.contains("line 59\n"),
        "{text}"
    );
    assert!(!text.contains("line 09"), "{text}");
    let tokens = fake.requests_to("POST", "/app/installations/42/access_tokens");
    let token = tokens
        .iter()
        .find(|token| token.body["permissions"].get("checks").is_some())
        .expect("a token for the check runs");
    assert_eq!(token.body["repository_ids"], json!([7]));
    assert_eq!(token.body["permissions"], json!({ "checks": "write" }));
    assert_eq!(report.mode(), ReportMode::CheckRun);
    assert_eq!(report.external_url, CHECK_PAGE);
    let statuses = format!("/repos/octo/app/statuses/{SHA}");
    assert!(fake.requests_to("POST", &statuses).is_empty());
    assert!(!logs.text().contains(GITHUB_TOKEN));
}

/// A cancelled run completes its check run with the conclusion `cancelled`, and the check run is
/// made one time only.
#[sqlx::test(migrations = false)]
async fn a_cancelled_run_changes_the_same_check_run(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let app = app_with(pool, &fake).await;
    let linked = linked(&app).await;

    // when
    let job = run_on_main(&app, &linked).await;
    wait_for_requests(&fake, "POST", CHECK_RUNS, 1).await;
    app.cancel_job(linked.owner, &job).await.unwrap();
    let report = app
        .wait_for_sent_state(linked.owner, &job, "cancelled")
        .await;

    // then
    let changes = fake.requests_to("PATCH", CHECK_RUN);
    let last = &changes.last().expect("a change of the check run").body;
    assert_eq!(last["conclusion"], "cancelled");
    assert_eq!(
        last["output"]["title"],
        "build was cancelled: 0/2 nodes passed"
    );
    assert_eq!(fake.requests_to("POST", CHECK_RUNS).len(), 1);
    assert_eq!(report.sent_state, "cancelled");
}

/// A link in the commit status mode posts commit statuses on GitHub, and no check run.
#[sqlx::test(migrations = false)]
async fn a_link_in_the_commit_status_mode_posts_statuses(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let statuses = format!("/repos/octo/app/statuses/{SHA}");
    fake.respond("POST", &statuses, 201, json!({ "id": 1 }), &[]);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app).await;
    app.set_link_report_mode(&linked.project, "commit_status")
        .await;

    // when
    let job = run_on_main(&app, &linked).await;
    let posted = wait_for_requests(&fake, "POST", &statuses, 1).await;
    let report = app.wait_for_sent_state(linked.owner, &job, "pending").await;

    // then
    assert_eq!(posted[0].body["context"], "scylla/build");
    assert_eq!(report.mode(), ReportMode::CommitStatus);
    assert!(fake.requests_to("POST", CHECK_RUNS).is_empty());
}
