//! The tests of the runs of the file pipelines that a person or a trigger starts: a run on a ref
//! reads the file at its commit, and a schedule, a webhook or a Git trigger runs the head of the
//! default branch.

use scylla_core::infra::vcs::sign_github;
use scylla_proto::pipelines::v1::git_ref::Kind;
use scylla_proto::pipelines::v1::{trigger, JobStatus, SourceEvent};
use scylla_testkit::{text, FakeProvider};
use serde_json::{json, Value};
use sqlx::PgPool;
use tonic::Code;

use super::{
    app_with, assert_reason, bench, files_project, jobs_of, linked, BUILD, BUILD_PATH, FEATURE_SHA,
    MAIN_SHA, NEXT_SHA,
};
use crate::assert_code;

/// A pull request adds a node to `build.toml`. A run on its branch takes the 5 nodes of the file
/// at its commit, with the file and the commit. A run with no ref takes the file at the head of
/// the default branch, and the pipeline keeps its 4 nodes. A commit with no file of the key gives
/// `NOT_FOUND`.
#[sqlx::test(migrations = false)]
async fn run_pipeline_runs_the_file_at_its_ref(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    fake.github_files("heads/main", MAIN_SHA, &[(BUILD_PATH, BUILD)]);
    fake.github_files("heads/feature", FEATURE_SHA, &[(BUILD_PATH, &bench())]);
    fake.github_files("heads/empty", NEXT_SHA, &[]);
    let app = app_with(pool, &fake).await;
    let files = files_project(&app, false).await;
    let (owner, build) = (files.linked.owner, files.build.as_str());

    // when
    let feature = Kind::Branch("feature".to_owned());
    let feature = app.run_pipeline_on(owner, build, feature).await.unwrap();
    let main = app.run_pipeline(owner, build).await.unwrap();
    let empty = Kind::Branch("empty".to_owned());
    let empty = app.run_pipeline_on(owner, build, empty).await;

    // then
    assert_eq!(feature.nodes.len(), 5);
    assert_eq!(feature.status(), JobStatus::Pending);
    assert!(feature.file_errors.is_empty());
    let file = feature.pipeline_file.as_ref().expect("a pipeline file");
    assert_eq!(
        (file.path.as_str(), file.commit_sha.as_str()),
        (BUILD_PATH, FEATURE_SHA)
    );
    let page = format!("https://github.com/octo/app/blob/{FEATURE_SHA}/{BUILD_PATH}");
    assert_eq!(file.url, page);
    let source = feature.source.as_ref().expect("a source");
    assert_eq!(source.event(), SourceEvent::Manual);
    assert_eq!(
        (source.sha.as_str(), source.branch.as_str()),
        (FEATURE_SHA, "feature")
    );
    assert_eq!(main.nodes.len(), 4);
    let source = main.source.as_ref().expect("a source");
    assert_eq!(
        (source.sha.as_str(), source.branch.as_str()),
        (MAIN_SHA, "main")
    );
    assert_eq!(main.pipeline_file.as_ref().unwrap().commit_sha, MAIN_SHA);
    assert_code(empty, Code::NotFound);
    assert_eq!(app.get_pipeline(owner, build).await.unwrap().nodes.len(), 4);
}

/// A schedule that a person fires now and a webhook of a file pipeline run its file at the head
/// of the default branch, with the trigger and the event `trigger`.
#[sqlx::test(migrations = false)]
async fn a_trigger_of_a_file_pipeline_runs_the_head_of_the_default_branch(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    fake.github_files("heads/main", MAIN_SHA, &[(BUILD_PATH, BUILD)]);
    fake.github_files("heads/main", NEXT_SHA, &[(BUILD_PATH, &bench())]);
    let app = app_with(pool, &fake).await;
    let files = files_project(&app, false).await;
    let (owner, build) = (files.linked.owner, files.build.as_str());
    let nightly = app.create_trigger(owner, build, "nightly", Some("0 3 * * *"), Vec::new());
    let nightly = text(&nightly.await.unwrap().trigger.unwrap().id).to_owned();
    let hook = app.create_trigger(owner, build, "hook", None, Vec::new());
    let hook = hook.await.unwrap();
    let (secret, hook) = (hook.webhook_secret, hook.trigger.unwrap());
    let Some(trigger::Kind::Webhook(webhook)) = &hook.kind else {
        panic!("a webhook: {hook:?}");
    };

    // when
    let fired = app.fire_trigger_now(owner, &nightly).await.unwrap();
    let body = "{}";
    let response = reqwest::Client::new()
        .post(format!("http://{}{}", app.addr, webhook.path))
        .header(
            "x-hub-signature-256",
            sign_github(secret.as_bytes(), body.as_bytes()),
        )
        .header("x-github-delivery", "hook-1")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();

    // then
    assert_eq!(fired.nodes.len(), 5);
    assert_eq!(text(&fired.trigger_id), nightly);
    let source = fired.source.as_ref().expect("a source");
    assert_eq!(source.event(), SourceEvent::Trigger);
    assert_eq!(
        (source.sha.as_str(), source.branch.as_str()),
        (NEXT_SHA, "main")
    );
    let file = fired.pipeline_file.as_ref().expect("a pipeline file");
    assert_eq!(file.commit_sha, NEXT_SHA);
    assert_eq!(response.status().as_u16(), 202);
    let answer: Value = response.json().await.unwrap();
    let job = answer["job_id"].as_str().expect("a job id");
    let posted = app.get_job(owner, job).await.unwrap();
    assert_eq!(posted.nodes.len(), 5);
    assert_eq!(text(&posted.trigger_id), text(&hook.id));
    assert_eq!(posted.source.unwrap().event(), SourceEvent::Trigger);
}

/// A Git trigger of an editor pipeline that a person fires now runs its pipeline on the head of
/// the default branch, with the trigger and the event `trigger`. A project with no link gives
/// `NO_REPOSITORY_LINK`.
#[sqlx::test(migrations = false)]
async fn fire_trigger_now_runs_a_git_trigger_on_the_head_of_the_default_branch(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    fake.github_files("heads/main", MAIN_SHA, &[]);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github").await;
    let alone = super::linked(&app, "none").await;
    let rules = json!({ "branches": ["main"] });
    let push = app.insert_git_trigger(&linked.editor, "on push", "push", rules.clone());
    let push = push.await;
    let unlinked = app.insert_git_trigger(&alone.editor, "on push", "push", rules);
    let unlinked = unlinked.await;

    // when
    let fired = app.fire_trigger_now(linked.owner, &push).await.unwrap();
    let refused = app.fire_trigger_now(alone.owner, &unlinked).await;

    // then
    assert_eq!(text(&fired.trigger_id), push);
    assert!(fired.pipeline_file.is_none());
    let source = fired.source.as_ref().expect("a source");
    assert_eq!(source.event(), SourceEvent::Trigger);
    let commit = (
        source.sha.as_str(),
        source.r#ref.as_str(),
        source.branch.as_str(),
    );
    assert_eq!(commit, (MAIN_SHA, "refs/heads/main", "main"));
    assert_reason(refused, "NO_REPOSITORY_LINK");
    let jobs = jobs_of(&app, alone.owner, &alone.editor).await;
    assert!(jobs.is_empty());
}
