//! The tests of the GitHub App of an instance: the manifest flow, the read of the App of the
//! settings or of the instance, its delete, and the access of its routes.

mod manifest;
mod removal;

use scylla_core::api::ServeOptions;
use scylla_domain::identity::actors::ActorId;
use scylla_proto::platform::v1::VcsApp;
use scylla_testkit::{FakeProvider, TestApp};
use serde_json::Value;
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

use crate::assert_code;

/// The code of the redirect of GitHub in the tests.
const CODE: &str = "c0de";

/// Starts the Community edition with GitHub on a fake, with the test App of the settings if
/// `with_app`.
async fn app_with_fake(pool: PgPool, with_app: bool) -> (TestApp, FakeProvider) {
    let fake = FakeProvider::start().await;
    let options = ServeOptions {
        vcs: fake.options(with_app),
        ..ServeOptions::default()
    };
    let app = TestApp::spawn_with(pool, scylla_ce::edition(), options).await;
    (app, fake)
}

/// Makes the App of the instance with the manifest flow as an instance admin. Returns the admin
/// and the App.
async fn created(app: &TestApp, fake: &FakeProvider) -> (ActorId, VcsApp) {
    let admin = app.admin().await;
    let started = app.start_github_manifest(admin, "acme").await.unwrap();
    fake.answer_manifest_code(CODE);
    let made = app
        .complete_github_manifest(admin, CODE, &state_of(&started.post_url))
        .await
        .unwrap();
    (admin, made)
}

/// Returns the state in a page of GitHub.
fn state_of(post_url: &str) -> String {
    let (_, query) = post_url.split_once("?state=").expect("a state");
    query.to_owned()
}

/// Reads the manifest of a start.
fn manifest_of(text: &str) -> Value {
    serde_json::from_str(text).expect("a JSON manifest")
}

/// Checks that a call failed with `FAILED_PRECONDITION` and this reason.
fn assert_reason<T: std::fmt::Debug>(result: Result<T, Status>, reason: &str) {
    let status = result.expect_err("the call must fail");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    let info = status.get_details_error_info().expect("ErrorInfo");
    assert_eq!(info.reason, reason, "{status:?}");
}

/// With the App of the settings, the server shows that App as managed, and the routes that change
/// it give `APP_MANAGED_BY_SETTINGS`.
#[sqlx::test(migrations = false)]
async fn get_vcs_app_shows_the_app_of_the_settings_as_managed(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool, true).await;
    let admin = app.admin().await;

    // when
    let managed = app.get_vcs_app(admin).await.unwrap().expect("an App");
    let started = app.start_github_manifest(admin, "").await;
    let completed = app.complete_github_manifest(admin, CODE, "x.y").await;
    let deleted = app.delete_vcs_app(admin).await;

    // then
    assert!(managed.managed_by_settings && managed.id.is_none());
    let ids = (managed.app_id.as_str(), managed.slug.as_str());
    assert_eq!(ids, ("1", "scylla-test"));
    let webhook = format!("http://{}/integrations/github/webhook", app.addr);
    assert_eq!(managed.webhook_url, webhook);
    assert!(managed.web_url.ends_with("/apps/scylla-test"));
    assert!(managed.last_delivery_at.is_none() && managed.created_at.is_none());
    assert_reason(started, "APP_MANAGED_BY_SETTINGS");
    assert_reason(completed, "APP_MANAGED_BY_SETTINGS");
    assert_reason(deleted, "APP_MANAGED_BY_SETTINGS");
}

/// Only the instance admins and a role of the platform reach the routes of the App: a user with
/// no role gets `PERMISSION_DENIED` on each one.
#[sqlx::test(migrations = false)]
async fn the_routes_of_the_app_need_the_platform(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool, false).await;
    let user = app.user().await;

    // when
    let got = app.get_vcs_app(user).await;
    let started = app.start_github_manifest(user, "").await;
    let completed = app.complete_github_manifest(user, CODE, "x.y").await;
    let deleted = app.delete_vcs_app(user).await;

    // then
    assert_code(got, Code::PermissionDenied);
    assert_code(started, Code::PermissionDenied);
    assert_code(completed, Code::PermissionDenied);
    assert_code(deleted, Code::PermissionDenied);
}

/// A server with no key of the secrets cannot keep the App: the start and the end of the
/// manifest flow give `SECRET_KEY_MISSING`.
#[sqlx::test(migrations = false)]
async fn the_manifest_flow_needs_the_key_of_the_secrets(pool: PgPool) {
    // given
    let app = TestApp::spawn_without_secret_key(pool, scylla_ce::edition()).await;
    let admin = app.admin().await;

    // when
    let started = app.start_github_manifest(admin, "").await;
    let completed = app.complete_github_manifest(admin, CODE, "x.y").await;

    // then
    assert_reason(started, "SECRET_KEY_MISSING");
    assert_reason(completed, "SECRET_KEY_MISSING");
}
