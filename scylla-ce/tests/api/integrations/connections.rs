//! The tests of the GitHub and GitLab connections of a workspace: the install of the GitHub
//! App, the GitLab tokens, and the reads, the delete and the access of the connections.

mod access;
mod github;
mod gitlab;
mod lists;

use scylla_core::api::ServeOptions;
use scylla_testkit::{FakeProvider, TestApp};
use serde_json::{json, Value};
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

/// Starts the Community edition with the test GitHub App, and with GitHub and GitLab on a fake.
async fn app_with_fake(pool: PgPool) -> (TestApp, FakeProvider) {
    let fake = FakeProvider::start().await;
    let options = ServeOptions {
        vcs: fake.options(true),
        ..ServeOptions::default()
    };
    let app = TestApp::spawn_with(pool, scylla_ce::edition(), options).await;
    (app, fake)
}

/// Checks that a call failed with `FAILED_PRECONDITION` and this reason.
fn assert_reason<T: std::fmt::Debug>(result: Result<T, Status>, reason: &str) {
    let status = result.expect_err("the call must fail");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    let info = status.get_details_error_info().expect("ErrorInfo");
    assert_eq!(info.reason, reason, "{status:?}");
}

/// The permissions of an installation of the test App, with no administration.
fn permissions() -> Value {
    json!({
        "checks": "write",
        "statuses": "write",
        "contents": "read",
        "pull_requests": "read",
        "metadata": "read",
    })
}

/// Returns the state in an install page.
fn state_of(install_url: &str) -> String {
    let (_, query) = install_url.split_once("?state=").expect("a state");
    query.to_owned()
}
