//! The tests of the check run of a run: one check run for each pipeline, made one time and then
//! changed, with its output. A GitHub link reports with check runs only.

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

/// A run that passes moves its check run to `in_progress` when the agent starts it, then
/// completes it with the conclusion `success`, a title with the nodes that passed, and no log
/// text. The check run is made one time only.
#[sqlx::test(migrations = false)]
async fn a_passed_run_on_github_completes_its_check_run(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let app = app_with(pool, &fake).await;
    let linked = linked(&app).await;
    let mut agent = app.run_agent(linked.owner, linked.workspace).await;

    // when
    let job = run_on_main(&app, &linked).await;
    let created = wait_for_requests(&fake, "POST", CHECK_RUNS, 1).await;
    let started = agent.start_next_job().await;
    app.wait_for_sent_state(linked.owner, &job, "in_progress")
        .await;
    agent.pass_job(&started, &["build", "test"]).await;
    let report = app.wait_for_sent_state(linked.owner, &job, "success").await;

    // then
    assert_eq!(started, job);
    assert_eq!(created[0].body["status"], "queued");
    let changes = fake.requests_to("PATCH", CHECK_RUN);
    let running = &changes.first().expect("a change of the check run").body;
    assert_eq!(running["status"], "in_progress");
    assert!(running.get("conclusion").is_none());
    let title = running["output"]["title"].as_str().unwrap();
    assert!(title.starts_with("build is running"), "{title}");
    let last = &changes.last().expect("the end of the check run").body;
    assert_eq!(last["status"], "completed");
    assert_eq!(last["conclusion"], "success");
    assert_eq!(last["output"]["title"], "build passed: 2/2 nodes passed");
    let summary = last["output"]["summary"].as_str().unwrap();
    assert!(summary.contains("| `build` | ✓ Passed |"), "{summary}");
    assert!(summary.contains("| `test` | ✓ Passed |"), "{summary}");
    assert!(last["output"].get("text").is_none());
    assert_eq!(fake.requests_to("POST", CHECK_RUNS).len(), 1);
    assert_eq!(report.sent_state, "success");
}
