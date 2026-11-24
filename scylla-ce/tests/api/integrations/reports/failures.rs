//! The tests of the failed calls of the reports: a limit of the provider, the last of the failed
//! calls, and a refusal that marks the connection and tells the owners and the admins.

use scylla_proto::integrations::v1::{ConnectionState, ReportState};
use scylla_testkit::{wait_for_requests, FakeProvider, Role};
use serde_json::json;
use sqlx::PgPool;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{app_with, github, linked, run_on_main, CHECK_RUNS};

/// A limit of the provider delays the report until the reset time. A retry sends it at once.
#[sqlx::test(migrations = false)]
async fn a_limit_of_the_provider_delays_the_report(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let reset = (now.as_secs() + 3600).to_string();
    let limited = [
        ("x-ratelimit-remaining", "0"),
        ("x-ratelimit-reset", reset.as_str()),
    ];
    let body = json!({ "message": "API rate limit exceeded" });
    fake.respond("POST", CHECK_RUNS, 403, body, &limited);
    github(&fake, 201);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github").await;

    // when
    let job = run_on_main(&app, &linked).await;
    let waiting = app
        .wait_for_report(linked.owner, &job, ReportState::Retrying)
        .await;
    let retried = app
        .retry_job_report(linked.owner, &job, "scylla/build")
        .await
        .unwrap();
    let sent = app
        .wait_for_report(linked.owner, &job, ReportState::Sent)
        .await;

    // then
    let next = waiting.next_attempt_at.expect("a time of the next call");
    assert_eq!(next.seconds.to_string(), reset);
    assert_eq!(waiting.attempts, 1);
    assert!(waiting.error.starts_with("GitHub: "), "{}", waiting.error);
    assert_eq!(retried.state(), ReportState::Sending);
    assert_eq!(retried.attempts, 0);
    assert_eq!(sent.attempts, 0);
    assert!(sent.error.is_empty());
    assert_eq!(
        wait_for_requests(&fake, "POST", CHECK_RUNS, 2).await.len(),
        2
    );
}

/// After the tenth failed call, the report stops with its error.
#[sqlx::test(migrations = false)]
async fn the_tenth_failed_call_stops_the_report(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github(&fake, 502);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github").await;

    // when
    let job = run_on_main(&app, &linked).await;
    app.wait_for_report(linked.owner, &job, ReportState::Retrying)
        .await;
    app.set_report_attempts(&job, 9).await;
    let stopped = app
        .wait_for_report(linked.owner, &job, ReportState::NotSent)
        .await;

    // then
    assert_eq!(stopped.attempts, 10);
    assert_eq!(stopped.error, "GitHub: the provider failed: status 502");
    assert!(stopped.next_attempt_at.is_none());
}

/// A refused call stops the report, marks the connection `needs_attention` with the reason, and
/// the owners and the admins of the workspace get a notice. A member gets none.
#[sqlx::test(migrations = false)]
async fn a_refused_call_marks_the_connection_and_tells_the_owners(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github(&fake, 404);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github").await;
    let workspace = linked.workspace.to_string();
    let (admin, member) = (app.user().await, app.user().await);
    for (actor, role) in [(admin, "admin"), (member, "member")] {
        let grant = Role::on("workspace", &workspace, role);
        app.grant_role(linked.owner, grant, &actor.to_string())
            .await
            .unwrap();
    }

    // when
    let job = run_on_main(&app, &linked).await;
    let stopped = app
        .wait_for_report(linked.owner, &job, ReportState::NotSent)
        .await;
    let connection = app
        .get_vcs_connection(linked.owner, &linked.connection)
        .await
        .unwrap();
    let owner_inbox = app.inbox(linked.owner).await;
    let admin_inbox = app.inbox(admin).await;
    let member_inbox = app.inbox(member).await;

    // then
    assert_eq!(stopped.error, "GitHub: the provider found no such object");
    assert_eq!(connection.state(), ConnectionState::NeedsAttention);
    assert_eq!(connection.state_reason, "APP_UNINSTALLED");
    let broken = |inbox: &[scylla_proto::notifications::v1::Notification]| {
        inbox
            .iter()
            .find(|row| row.kind == "vcs.connection_broken")
            .cloned()
    };
    let owner_row = broken(&owner_inbox).expect("a notice for the owner");
    assert_eq!(owner_row.reason, "owner");
    assert_eq!(owner_row.params["reason"], "APP_UNINSTALLED");
    assert_eq!(owner_row.params["provider"], "github");
    assert_eq!(owner_row.params["account"], "octo");
    let admin_row = broken(&admin_inbox).expect("a notice for the admin");
    assert_eq!(admin_row.reason, "workspace_admin");
    assert!(broken(&member_inbox).is_none());
}
