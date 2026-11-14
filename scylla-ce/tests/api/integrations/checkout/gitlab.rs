//! The tests of the GitLab deploy tokens of the checkouts: one for each dispatch, with
//! `read_repository` only, deleted when the job ends.

use std::time::Duration;

use scylla_proto::agents::v1::EventKind;
use scylla_proto::pipelines::v1::git_ref::Kind;
use scylla_testkit::{text, FakeProvider, TestApp};
use sqlx::PgPool;

use super::{app_with, basic, gitlab, linked, DEPLOY_TOKEN, GITLAB_TOKEN, SHA};

/// Waits until the fake got a delete of the token 99 and no row of a deploy token stays, for at
/// most 20 seconds.
async fn deleted(app: &TestApp, fake: &FakeProvider) -> bool {
    let path = "/api/v4/projects/7/deploy_tokens/99";
    for _ in 0..200 {
        let called = !fake.requests_to("DELETE", path).is_empty();
        if called && app.checkout_tokens().await.is_empty() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// A dispatch on GitLab makes a deploy token with `read_repository` that ends after the longest
/// run of the job, and keeps its row. The header has the user and the token.
#[sqlx::test(migrations = false)]
async fn dispatch_makes_a_deploy_token_of_the_job(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    gitlab(&fake);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "gitlab", None).await;
    let main = Kind::Branch("main".to_owned());
    let job = app.run_pipeline_on(linked.owner, &linked.pipeline, main);
    let job = text(&job.await.unwrap().id).to_owned();

    // when
    let (dispatch, _up) = app.first_dispatch(linked.owner, linked.workspace).await;

    // then
    let checkout = dispatch.checkout.expect("a checkout");
    assert_eq!(checkout.clone_url, "https://gitlab.com/group/app.git");
    assert_eq!(checkout.sha, SHA);
    let header = basic("gitlab+deploy-token-99", DEPLOY_TOKEN);
    assert_eq!(checkout.auth_header, header);
    let made = fake.requests_to("POST", "/api/v4/projects/7/deploy_tokens");
    let made = made.last().expect("a deploy token request");
    assert_eq!(made.header("private-token"), Some(GITLAB_TOKEN));
    assert_eq!(made.body["scopes"], serde_json::json!(["read_repository"]));
    assert_eq!(made.body["name"], format!("scylla-job-{job}"));
    let expiry = made.body["expires_at"].as_str().expect("an expiry");
    assert!(expiry.ends_with('Z') && expiry.len() == 20, "{expiry}");
    let rows = app.checkout_tokens().await;
    assert_eq!(rows.len(), 1);
    let (owner, token, life) = &rows[0];
    assert_eq!(
        (owner.as_deref(), token.as_str()),
        (Some(job.as_str()), "99")
    );
    assert!((7140..=7200).contains(life), "{life}");
}

/// The end of the job deletes its deploy token on GitLab, then its row.
#[sqlx::test(migrations = false)]
async fn the_end_of_a_job_deletes_its_deploy_token(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    gitlab(&fake);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "gitlab", None).await;
    let main = Kind::Branch("main".to_owned());
    let job = app.run_pipeline_on(linked.owner, &linked.pipeline, main);
    let job = text(&job.await.unwrap().id).to_owned();
    let mut agent = app
        .agent_with(linked.owner, linked.workspace, &["checkout"])
        .await;
    agent.dispatch_in(Duration::from_secs(10)).await.unwrap();
    let kept = app.checkout_tokens().await;

    // when
    agent.event(&job, EventKind::JobStarted, "").await;
    agent.event(&job, EventKind::JobFailed, "stopped").await;

    // then
    assert_eq!(kept.len(), 1);
    assert!(deleted(&app, &fake).await, "{:?}", fake.requests());
    let call = fake.requests_to("DELETE", "/api/v4/projects/7/deploy_tokens/99");
    assert_eq!(call[0].header("private-token"), Some(GITLAB_TOKEN));
}

/// A job that a person cancels loses its deploy token too.
#[sqlx::test(migrations = false)]
async fn a_cancelled_job_loses_its_deploy_token(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    gitlab(&fake);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "gitlab", None).await;
    let main = Kind::Branch("main".to_owned());
    let job = app.run_pipeline_on(linked.owner, &linked.pipeline, main);
    let job = text(&job.await.unwrap().id).to_owned();
    let (_dispatch, _up) = app.first_dispatch(linked.owner, linked.workspace).await;

    // when
    app.cancel_job(linked.owner, &job).await.unwrap();

    // then
    assert!(deleted(&app, &fake).await, "{:?}", fake.requests());
}
