//! The tests of the commit statuses of a run on GitHub and on GitLab: the states, the name, the
//! description and the link of each report, and the runs that report nothing.

use scylla_proto::integrations::v1::{ReportMode, ReportState, VcsProvider};
use scylla_testkit::{wait_for_requests, FakeProvider, Logs};
use sqlx::PgPool;

use super::{
    app_with, github, gitlab, linked, run_on_main, GITHUB_STATUSES, GITHUB_TOKEN, GITLAB_STATUSES,
    GITLAB_TOKEN, SHA,
};

/// A run on a commit reports `pending` when it starts, then its final state, with the name of
/// its pipeline, a short description and the link of the run. The token of the installation is
/// narrowed to the repository and to the statuses, and no token goes to a log or an answer.
#[sqlx::test(migrations = false)]
async fn a_run_on_github_reports_its_states_on_its_commit(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github(&fake, 201);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github").await;
    let logs = Logs::json();

    // when
    let job = run_on_main(&app, &linked).await;
    let started = wait_for_requests(&fake, "POST", GITHUB_STATUSES, 1).await;
    app.cancel_job(linked.owner, &job).await.unwrap();
    let ended = wait_for_requests(&fake, "POST", GITHUB_STATUSES, 2).await;
    let report = app
        .wait_for_report(linked.owner, &job, ReportState::Sent)
        .await;

    // then
    let first = &started[0];
    let page = format!(
        "/projects/{}/pipelines/{}/jobs/{job}",
        linked.project, linked.pipeline
    );
    assert_eq!(first.body["state"], "pending");
    assert_eq!(first.body["context"], "scylla/build");
    assert_eq!(first.body["description"], "Queued");
    assert!(first.body["target_url"].as_str().unwrap().ends_with(&page));
    assert_eq!(first.header("authorization"), Some("Bearer ghs_reports"));
    let last = ended.last().unwrap();
    assert_eq!(last.body["state"], "error");
    assert_eq!(last.body["description"], "Cancelled");
    let tokens = fake.requests_to("POST", "/app/installations/42/access_tokens");
    let token = tokens
        .iter()
        .find(|token| token.body["permissions"].get("statuses").is_some())
        .expect("a token for the statuses");
    assert_eq!(token.body["repository_ids"], serde_json::json!([7]));
    assert_eq!(
        token.body["permissions"],
        serde_json::json!({ "statuses": "write" })
    );
    assert_eq!(report.context, "scylla/build");
    assert_eq!(report.provider(), VcsProvider::Github);
    assert_eq!(report.mode(), ReportMode::CommitStatus);
    assert_eq!(report.sent_state, "error");
    assert_eq!(
        report.external_url,
        format!("https://github.com/octo/app/commit/{SHA}")
    );
    let answer = format!("{report:?}");
    let text = logs.text();
    assert!(!text.contains(GITHUB_TOKEN) && !answer.contains(GITHUB_TOKEN));
}

/// A report never goes back from a final state: no `pending` comes after the end of the run.
#[sqlx::test(migrations = false)]
async fn a_report_sends_no_open_state_after_a_final_one(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github(&fake, 201);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github").await;

    // when
    let job = run_on_main(&app, &linked).await;
    app.cancel_job(linked.owner, &job).await.unwrap();
    app.wait_for_report(linked.owner, &job, ReportState::Sent)
        .await;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // then
    let states: Vec<String> = fake
        .requests_to("POST", GITHUB_STATUSES)
        .iter()
        .map(|request| {
            request.body["state"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    let end = states.iter().position(|state| state == "error");
    assert_eq!(end, Some(states.len() - 1), "{states:?}");
}

/// On GitLab, the service account posts each state with the name of the report and the branch,
/// and each later state goes to the GitLab pipeline of the first one.
#[sqlx::test(migrations = false)]
async fn a_run_on_gitlab_reports_as_the_service_account(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    gitlab(&fake);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "gitlab").await;

    // when
    let job = run_on_main(&app, &linked).await;
    wait_for_requests(&fake, "POST", GITLAB_STATUSES, 1).await;
    app.cancel_job(linked.owner, &job).await.unwrap();
    let requests = wait_for_requests(&fake, "POST", GITLAB_STATUSES, 2).await;
    let report = app
        .wait_for_report(linked.owner, &job, ReportState::Sent)
        .await;

    // then
    for request in &requests {
        assert_eq!(request.header("private-token"), Some(GITLAB_TOKEN));
        assert_eq!(request.body["name"], "scylla/build");
        assert_eq!(request.body["ref"], "main");
    }
    assert_eq!(requests[0].body["state"], "pending");
    assert!(requests[0].body.get("pipeline_id").is_none());
    assert_eq!(requests[1].body["state"], "canceled");
    assert_eq!(requests[1].body["pipeline_id"], 77);
    assert_eq!(report.mode(), ReportMode::CommitStatus);
    assert_eq!(report.sent_state, "canceled");
    assert_eq!(
        report.external_url,
        "https://gitlab.com/group/app/-/pipelines/77"
    );
}

/// A run with no commit, a commit with no link, and a link with the reports off report
/// nothing, and the list tells why.
#[sqlx::test(migrations = false)]
async fn a_run_with_no_report_tells_why(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github(&fake, 201);
    let app = app_with(pool, &fake).await;
    let plain = linked(&app, "none").await;
    let off = linked(&app, "github").await;
    app.set_link_reports(&off.project, false).await;

    // when
    let no_source = app
        .run_pipeline(plain.owner, &plain.pipeline)
        .await
        .unwrap();
    let no_source = scylla_testkit::text(&no_source.id).to_owned();
    let no_link = app
        .run_pipeline(plain.owner, &plain.pipeline)
        .await
        .unwrap();
    let no_link = scylla_testkit::text(&no_link.id).to_owned();
    app.insert_job_source(&no_link, SHA, "main", "push").await;
    let reports_off = run_on_main(&app, &off).await;

    // then
    let reasons = app
        .list_job_reports(plain.owner, &[&no_source, &no_link])
        .await
        .unwrap()
        .jobs;
    let off_reports = app.job_reports(off.owner, &reports_off).await;
    assert_eq!(reasons[0].no_report_reason, "NO_SOURCE");
    assert_eq!(reasons[1].no_report_reason, "NO_REPOSITORY_LINK");
    assert!(reasons.iter().all(|job| job.reports.is_empty()));
    assert_eq!(off_reports.no_report_reason, "REPORTS_OFF");
    assert!(off_reports.reports.is_empty());
}
