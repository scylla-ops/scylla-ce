//! The tests of a check run after an uninstall of the GitHub App during its run (story 8.3 of the
//! research): the report that cannot go out, then a new install on the same account and a retry
//! that sends the end of the run.

use scylla_proto::integrations::v1::{ConnectionState, ReportState};
use scylla_testkit::{text, wait_for_requests, FakeProvider, RunAgent, TestApp};
use serde_json::{json, Value};
use sqlx::PgPool;

use super::{
    app_with, linked, post_github, run_on_main, Linked, CHECK_PAGE, CHECK_RUN, CHECK_RUNS,
};

/// The authorization of the calls with the token of the new installation 43, as
/// `FakeProvider::answer_github_install` gives it.
const NEW_TOKEN: &str = "Bearer ghs_installation";

/// Returns the `installation` delivery of the uninstall of the installation 42.
fn uninstalled() -> Value {
    json!({
        "action": "deleted",
        "installation": { "id": 42, "permissions": {} },
        "sender": { "login": "octo-owner" },
    })
}

/// Returns the permissions of the new installation: all that the reports need.
fn permissions() -> Value {
    json!({
        "checks": "write",
        "statuses": "write",
        "contents": "read",
        "pull_requests": "read",
        "metadata": "read",
    })
}

/// Starts the run of a linked project, and waits until its check run is in progress on GitHub.
/// Returns the job.
async fn running(
    app: &TestApp,
    fake: &FakeProvider,
    linked: &Linked,
    agent: &mut RunAgent,
) -> String {
    let job = run_on_main(app, linked).await;
    wait_for_requests(fake, "POST", CHECK_RUNS, 1).await;
    let started = agent.start_next_job().await;
    assert_eq!(started, job);
    app.wait_for_sent_state(linked.owner, &job, "in_progress")
        .await;
    job
}

/// Installs the App again on the account `octo` of the link, as the installation 43 that sees
/// the repository of the link.
async fn install_again(app: &TestApp, fake: &FakeProvider, linked: &Linked) {
    fake.answer_github_install(43, "octo", permissions(), &[(7, "octo/app", false)]);
    let workspace = linked.workspace.to_string();
    let connection = app.install_github(linked.owner, &workspace, 43).await;
    assert_eq!(connection.unwrap().external_id, "43");
}

/// The story: an owner uninstalls the App while a run runs. The end of the run gets 404, so its
/// report is not sent, the owner gets a notice, and the uninstall webhook disconnects the
/// connection. A new install on the same account takes over the connection, its link is active
/// again, and "Retry now" sends the final state with a token of the new installation.
#[sqlx::test(migrations = false)]
async fn retry_job_report_sends_the_end_of_the_run_after_a_new_install(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let app = app_with(pool, &fake).await;
    let check = json!({ "id": 99, "html_url": CHECK_PAGE });
    let gone = json!({ "message": "Not Found" });
    fake.respond("PATCH", CHECK_RUN, 404, gone, &[]);
    fake.respond("PATCH", CHECK_RUN, 200, check, &[]);
    let linked = linked(&app).await;
    let link = app.get_repository_link(linked.owner, &linked.project).await;
    let connection = text(&link.unwrap().expect("a link").connection_id).to_owned();
    let mut agent = app.run_agent(linked.owner, linked.workspace).await;
    let job = running(&app, &fake, &linked, &mut agent).await;

    // when
    agent.pass_job(&job, &["build", "test"]).await;
    let refused = app
        .wait_for_report(linked.owner, &job, ReportState::NotSent)
        .await;
    post_github(&app, "installation", "d-1", &uninstalled()).await;
    let disconnected = app
        .get_vcs_connection(linked.owner, &connection)
        .await
        .unwrap();
    let inbox = app.inbox(linked.owner).await;
    install_again(&app, &fake, &linked).await;
    let link = app
        .get_repository_link(linked.owner, &linked.project)
        .await
        .unwrap()
        .expect("a link");
    let retried = app
        .retry_job_report(linked.owner, &job, "scylla/build")
        .await
        .unwrap();
    let sent = app.wait_for_sent_state(linked.owner, &job, "success").await;

    // then
    assert_eq!(refused.error, "GitHub: the provider found no such object");
    assert_eq!(refused.sent_state, "in_progress");
    assert_eq!(disconnected.state(), ConnectionState::Disconnected);
    assert_eq!(disconnected.state_reason, "APP_UNINSTALLED");
    let notice = inbox
        .iter()
        .find(|row| row.kind == "vcs.connection_broken")
        .expect("a notice for the owner");
    assert_eq!(notice.params["reason"], "APP_UNINSTALLED");
    assert_eq!(text(&link.connection_id), connection);
    assert_eq!(link.state(), ConnectionState::Active);
    assert_eq!(retried.state(), ReportState::Sending);
    assert_eq!(sent.state(), ReportState::Sent);
    let changes = fake.requests_to("PATCH", CHECK_RUN);
    assert_eq!(changes.len(), 3);
    let last = changes.last().expect("the retry");
    assert_eq!(last.header("authorization"), Some(NEW_TOKEN));
    assert_eq!(last.body["status"], "completed");
    assert_eq!(last.body["conclusion"], "success");
    assert_eq!(fake.requests_to("POST", CHECK_RUNS).len(), 1);
}

/// A run that ends after the uninstall webhook keeps its final state as not sent, with no call
/// to GitHub. After a new install on the same account, "Retry now" sends this final state, not
/// the old one.
#[sqlx::test(migrations = false)]
async fn a_run_that_ends_while_the_app_is_uninstalled_keeps_its_final_state(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let app = app_with(pool, &fake).await;
    let linked = linked(&app).await;
    let mut agent = app.run_agent(linked.owner, linked.workspace).await;
    let job = running(&app, &fake, &linked, &mut agent).await;
    post_github(&app, "installation", "d-1", &uninstalled()).await;

    // when
    agent.pass_job(&job, &["build", "test"]).await;
    let stopped = app
        .wait_for_report(linked.owner, &job, ReportState::NotSent)
        .await;
    let calls = fake.requests_to("PATCH", CHECK_RUN).len();
    install_again(&app, &fake, &linked).await;
    app.retry_job_report(linked.owner, &job, "scylla/build")
        .await
        .unwrap();
    let sent = app.wait_for_sent_state(linked.owner, &job, "success").await;

    // then
    assert_eq!(
        stopped.error,
        "the repository link or its connection is not active"
    );
    assert_eq!(stopped.sent_state, "in_progress");
    assert_eq!(calls, 1);
    assert_eq!(sent.state(), ReportState::Sent);
    let changes = fake.requests_to("PATCH", CHECK_RUN);
    assert_eq!(changes.len(), 2);
    let last = changes.last().expect("the retry");
    assert_eq!(last.header("authorization"), Some(NEW_TOKEN));
    assert_eq!(last.body["status"], "completed");
    assert_eq!(last.body["conclusion"], "success");
}
