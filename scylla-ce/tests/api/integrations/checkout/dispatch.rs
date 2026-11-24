//! The tests of the dispatch of a job with a checkout: the agents that get it, the token of one
//! repository, the ref of a fork, and a link that cannot serve.

use std::time::Duration;

use scylla_proto::pipelines::v1::git_ref::Kind;
use scylla_proto::pipelines::v1::{JobStatus, NodeStatus, UnfitReason};
use scylla_testkit::{text, FakeProvider, Logs};
use serde_json::json;
use sqlx::PgPool;

use super::{app_with, basic, github, job_when, linked, GITHUB_TOKEN, SHA};

/// Returns the branch `main`.
fn main() -> Kind {
    Kind::Branch("main".to_owned())
}

/// An agent with `checkout` gets the commit, the clone URL and a header with an installation
/// token narrowed to the one repository and to `contents: read`. The token is in no log line.
#[sqlx::test(migrations = false)]
async fn dispatch_gives_a_token_of_one_repository(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github(&fake);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github", None).await;
    let job = app.run_pipeline_on(linked.owner, &linked.pipeline, main());
    let job = text(&job.await.unwrap().id).to_owned();
    let _under = Logs::pretty();
    let logs = Logs::json();

    // when
    let mut agent = app
        .agent_with(linked.owner, linked.workspace, &["checkout"])
        .await;
    let dispatch = agent.dispatch_in(Duration::from_secs(10)).await;
    let read = app.get_job(linked.owner, &job).await.unwrap();

    // then
    let checkout = dispatch.expect("a dispatch").checkout.expect("a checkout");
    assert_eq!(checkout.clone_url, "https://github.com/octo/app.git");
    assert_eq!(
        (checkout.sha.as_str(), checkout.fetch_ref.as_str()),
        (SHA, "")
    );
    assert_eq!(checkout.auth_header, basic("x-access-token", GITHUB_TOKEN));
    assert_eq!(
        (checkout.depth, checkout.submodules, checkout.lfs),
        (1, false, false)
    );
    let asked = fake.requests_to("POST", "/app/installations/42/access_tokens");
    let scope = json!({ "repository_ids": [7], "permissions": { "contents": "read" } });
    assert_eq!(asked.last().expect("a token request").body, scope);
    let run = read.checkout.expect("a checkout run");
    assert_eq!(run.status, NodeStatus::Pending as i32);
    assert_eq!(read.waiting_reason, "");
    let text = logs.text();
    assert!(text.contains("job.dispatched"), "{text}");
    for secret in [GITHUB_TOKEN, checkout.auth_header.as_str()] {
        assert!(!text.contains(secret), "{secret} is in the logs:\n{text}");
    }
}

/// An agent with no `checkout` never gets a job with a checkout, and the job tells why it waits.
/// It still gets a job with no source. An agent with `checkout` then gets the job.
#[sqlx::test(migrations = false)]
async fn an_old_agent_never_gets_a_job_with_a_checkout(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github(&fake);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github", None).await;
    let job = app.run_pipeline_on(linked.owner, &linked.pipeline, main());
    let job = text(&job.await.unwrap().id).to_owned();
    let mut old = app.agent_with(linked.owner, linked.workspace, &[]).await;

    // when
    let nothing = old.dispatch_in(Duration::from_secs(2)).await;
    let waiting = app.get_job(linked.owner, &job).await.unwrap();
    let plain = app.run_pipeline(linked.owner, &linked.pipeline).await;
    let plain = text(&plain.unwrap().id).to_owned();
    let old_gets = old.dispatch_in(Duration::from_secs(10)).await;
    let mut new = app
        .agent_with(linked.owner, linked.workspace, &["checkout"])
        .await;
    let new_gets = new.dispatch_in(Duration::from_secs(10)).await;
    let taken = app.get_job(linked.owner, &job).await.unwrap();

    // then
    assert!(nothing.is_none(), "{nothing:?}");
    assert_eq!(waiting.waiting_reason, "NO_AGENT_WITH_CHECKOUT");
    let old_gets = old_gets.expect("the job with no source");
    assert_eq!(old_gets.job_id.unwrap().value, plain);
    assert!(old_gets.checkout.is_none());
    let new_gets = new_gets.expect("the job with a checkout");
    assert_eq!(new_gets.job_id.unwrap().value, job);
    assert!(new_gets.checkout.is_some());
    assert_eq!(taken.waiting_reason, "");
}

/// A job that waits for an agent with `checkout` names the online agents that cannot get the
/// code: an agent with an old protocol must be updated, and a new one with no git needs git.
#[sqlx::test(migrations = false)]
async fn a_waiting_job_names_the_agents_that_cannot_get_the_code(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github(&fake);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github", None).await;
    let job = app.run_pipeline_on(linked.owner, &linked.pipeline, main());
    let job = text(&job.await.unwrap().id).to_owned();
    let old = app.agent_at(linked.owner, linked.workspace, &[], 0).await;
    let no_git = app.agent_at(linked.owner, linked.workspace, &[], 2).await;

    // when
    let waiting = app.get_job(linked.owner, &job).await.unwrap();

    // then
    assert_eq!(waiting.waiting_reason, "NO_AGENT_WITH_CHECKOUT");
    let mut unfit: Vec<(String, i32)> = waiting
        .unfit_agents
        .iter()
        .map(|agent| (text(&agent.id).to_owned(), agent.reason))
        .collect();
    unfit.sort();
    let mut expected = vec![
        (old.key.app.clone(), UnfitReason::UpdateAgent as i32),
        (no_git.key.app.clone(), UnfitReason::InstallGit as i32),
    ];
    expected.sort();
    assert_eq!(unfit, expected);
    assert!(waiting
        .unfit_agents
        .iter()
        .all(|agent| agent.version == "1.0.0"));
}

/// A job from a fork fetches the ref of its pull request from the base repository, with a token
/// narrowed to the one repository and to `contents: read`.
#[sqlx::test(migrations = false)]
async fn a_job_from_a_fork_fetches_the_ref_of_its_pull_request(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github(&fake);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github", None).await;
    let job = app.run_pipeline_on(linked.owner, &linked.pipeline, main());
    let job = text(&job.await.unwrap().id).to_owned();
    app.set_job_source(&job, "evil", Some(57), Some("evil/app"))
        .await;

    // when
    let (dispatch, _up) = app.first_dispatch(linked.owner, linked.workspace).await;

    // then
    let checkout = dispatch.checkout.expect("a checkout");
    assert_eq!(checkout.clone_url, "https://github.com/octo/app.git");
    assert_eq!(checkout.fetch_ref, "refs/pull/57/head");
    assert_eq!(checkout.sha, SHA);
    assert_eq!(checkout.auth_header, basic("x-access-token", GITHUB_TOKEN));
    let asked = fake.requests_to("POST", "/app/installations/42/access_tokens");
    let scope = json!({ "repository_ids": [7], "permissions": { "contents": "read" } });
    assert_eq!(asked.last().expect("a token request").body, scope);
}

/// A job whose project lost its link fails at its dispatch with the reason, and no agent gets
/// it.
#[sqlx::test(migrations = false)]
async fn a_job_with_no_link_fails_at_its_dispatch(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github(&fake);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github", None).await;
    let job = app.run_pipeline_on(linked.owner, &linked.pipeline, main());
    let job = text(&job.await.unwrap().id).to_owned();
    sqlx::query("DELETE FROM repository_links WHERE project_id = $1::uuid")
        .bind(&linked.project)
        .execute(&app.pool)
        .await
        .unwrap();

    // when
    let mut agent = app
        .agent_with(linked.owner, linked.workspace, &["checkout"])
        .await;
    let dispatch = agent.dispatch_in(Duration::from_secs(2)).await;
    let failed = job_when(&app, linked.owner, &job, |job| {
        job.status == JobStatus::Failed as i32
    })
    .await;

    // then
    assert!(dispatch.is_none(), "{dispatch:?}");
    let error = "the checkout cannot start: the project has no linked repository";
    assert_eq!(failed.error, error);
    let run = failed.checkout.expect("a checkout run");
    assert_eq!(run.status, NodeStatus::Cancelled as i32);
}
