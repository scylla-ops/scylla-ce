//! The tests of the runs of the pushes, the tags and the pull requests of a branch of the
//! repository: the triggers that match, and one run for each pipeline and commit.

use scylla_proto::pipelines::v1::{git_ref, FireStatus, JobStatus, SourceEvent};
use scylla_testkit::text;
use serde_json::json;
use sqlx::PgPool;

use super::{
    app_with_fake, deliver_github, github_pull, github_push, jobs_of, linked, pipeline_with,
    OTHER_SHA, SHA,
};

/// A push or a tag that matches an enabled Git trigger starts one run of its pipeline with the
/// commit, as the runner of the triggers. A ref that no rule matches and a disabled trigger start
/// nothing, and the fire is recorded on the trigger.
#[sqlx::test(migrations = false)]
async fn a_push_or_a_tag_that_matches_a_trigger_starts_one_run(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    let push = [("push", json!({ "branches": ["main"] }))];
    let (build, build_triggers) = pipeline_with(&app, &linked, &push).await;
    let tag = [("tag", json!({ "patterns": ["v*"] }))];
    let (release, _) = pipeline_with(&app, &linked, &tag).await;
    let (off, off_triggers) = pipeline_with(&app, &linked, &[("push", json!({}))]).await;
    let off_trigger = &off_triggers[0];
    app.set_trigger_enabled(linked.owner, off_trigger, false)
        .await
        .unwrap();

    // when
    let main = github_push("refs/heads/main", SHA, "Fix the build");
    let done = deliver_github(&app, "push", "d-1", &main).await;
    let nightly = github_push("refs/tags/nightly", SHA, "Fix the build");
    deliver_github(&app, "push", "d-2", &nightly).await;
    let version = github_push("refs/tags/v1.0.0", OTHER_SHA, "Release");
    deliver_github(&app, "push", "d-3", &version).await;

    // then
    assert_eq!(done.error, "");
    let runner = app.trigger_runner(linked.workspace).await.unwrap();
    let jobs = jobs_of(&app, &linked, &build).await;
    assert_eq!(jobs.len(), 1);
    let job = &jobs[0];
    assert_eq!(job.status(), JobStatus::Pending);
    assert_eq!(text(&job.trigger_id), build_triggers[0]);
    assert_eq!(text(&job.created_by), runner);
    let source = job.source.as_ref().unwrap();
    assert_eq!(source.event(), SourceEvent::Push);
    let commit = (
        source.sha.as_str(),
        source.r#ref.as_str(),
        source.branch.as_str(),
    );
    assert_eq!(commit, (SHA, "refs/heads/main", "main"));
    assert_eq!(source.message_title, "Fix the build");
    let releases = jobs_of(&app, &linked, &release).await;
    assert_eq!(releases.len(), 1);
    let source = releases[0].source.as_ref().unwrap();
    assert_eq!(source.event(), SourceEvent::Tag);
    assert_eq!(
        (source.sha.as_str(), source.tag.as_str()),
        (OTHER_SHA, "v1.0.0")
    );
    assert!(jobs_of(&app, &linked, &off).await.is_empty());
    let trigger = app.get_trigger(linked.owner, &build_triggers[0]).await;
    assert_eq!(trigger.unwrap().last_status(), FireStatus::Ok);
}

/// A developer pushes a branch, and the push starts a run. Then she opens a pull request from
/// that branch: the pull request goes into the source of the same run, and nothing new starts. A
/// manual run on the commit is a new run.
#[sqlx::test(migrations = false)]
async fn a_pull_request_on_the_commit_of_a_push_joins_its_run(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    let rules = [
        ("push", json!({})),
        ("pull_request", json!({ "branches": ["main"] })),
    ];
    let (pipeline, _) = pipeline_with(&app, &linked, &rules).await;
    let push = github_push("refs/heads/feature/login", SHA, "Add login");
    deliver_github(&app, "push", "d-1", &push).await;

    // when
    let pull = github_pull("opened", 3, SHA, (7, "octo/app"), false);
    let done = deliver_github(&app, "pull_request", "d-2", &pull).await;

    // then
    assert_eq!(done.error, "");
    let jobs = jobs_of(&app, &linked, &pipeline).await;
    assert_eq!(jobs.len(), 1);
    let source = jobs[0].source.as_ref().unwrap();
    assert_eq!(source.event(), SourceEvent::Push);
    assert_eq!(source.branch, "feature/login");
    let pulls: Vec<_> = source
        .pull_requests
        .iter()
        .map(|pull| (pull.number, pull.target_branch.as_str()))
        .collect();
    assert_eq!(pulls, vec![(3, "main")]);
    fake.answer_installation_token();
    fake.answer_github_commit("octo/app", SHA, "Add login");
    let kind = git_ref::Kind::Sha(SHA.to_owned());
    let manual = app.run_pipeline_on(linked.owner, &pipeline, kind).await;
    assert_eq!(manual.unwrap().source.unwrap().event(), SourceEvent::Manual);
    assert_eq!(jobs_of(&app, &linked, &pipeline).await.len(), 2);
}

/// A push with `[skip ci]` in its message, a draft pull request and a closed pull request start
/// no run. The next push starts one.
#[sqlx::test(migrations = false)]
async fn skip_ci_drafts_and_closed_pull_requests_start_no_run(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    let rules = [
        ("push", json!({ "branches": ["main"] })),
        ("pull_request", json!({})),
    ];
    let (pipeline, _) = pipeline_with(&app, &linked, &rules).await;

    // when
    let skipped = github_push("refs/heads/main", SHA, "Fix the docs [skip ci]");
    deliver_github(&app, "push", "d-1", &skipped).await;
    let draft = github_pull("opened", 4, SHA, (7, "octo/app"), true);
    deliver_github(&app, "pull_request", "d-2", &draft).await;
    let closed = github_pull("closed", 4, SHA, (7, "octo/app"), false);
    deliver_github(&app, "pull_request", "d-3", &closed).await;
    let next = github_push("refs/heads/main", OTHER_SHA, "Fix the build");
    deliver_github(&app, "push", "d-4", &next).await;

    // then
    let jobs = jobs_of(&app, &linked, &pipeline).await;
    let shas: Vec<_> = jobs
        .iter()
        .map(|job| job.source.as_ref().unwrap().sha.as_str())
        .collect();
    assert_eq!(shas, vec![OTHER_SHA]);
}

/// A push and a pull request on one commit at the same time make one run, with the branch and
/// the pull request in its source.
#[sqlx::test(migrations = false)]
async fn two_events_on_one_commit_at_one_time_make_one_run(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    let rules = [("push", json!({})), ("pull_request", json!({}))];
    let (pipeline, _) = pipeline_with(&app, &linked, &rules).await;
    let push = github_push("refs/heads/feature/login", SHA, "Add login");
    let pull = github_pull("opened", 3, SHA, (7, "octo/app"), false);

    // when
    let (pushed, pulled) = tokio::join!(
        deliver_github(&app, "push", "d-1", &push),
        deliver_github(&app, "pull_request", "d-2", &pull),
    );

    // then
    assert_eq!((pushed.error.as_str(), pulled.error.as_str()), ("", ""));
    let jobs = jobs_of(&app, &linked, &pipeline).await;
    assert_eq!(jobs.len(), 1);
    let source = jobs[0].source.as_ref().unwrap();
    assert_eq!(source.branch, "feature/login");
    let numbers: Vec<_> = source
        .pull_requests
        .iter()
        .map(|pull| pull.number)
        .collect();
    assert_eq!(numbers, vec![3]);
}

/// A pull request from a branch of the repository starts a run on its head commit, as the runner
/// of its trigger. A new commit of the pull request starts a new run on the new head, and the run
/// of the old commit stays.
#[sqlx::test(migrations = false)]
async fn a_pull_request_from_a_branch_runs_each_new_head(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    let rules = [("pull_request", json!({ "branches": ["main"] }))];
    let (pipeline, triggers) = pipeline_with(&app, &linked, &rules).await;
    let opened = github_pull("opened", 42, SHA, (7, "octo/app"), false);
    deliver_github(&app, "pull_request", "d-1", &opened).await;

    // when
    let fixed = "fedcba9876543210fedcba9876543210fedcba98";
    let pushed = github_pull("synchronize", 42, fixed, (7, "octo/app"), false);
    let done = deliver_github(&app, "pull_request", "d-2", &pushed).await;

    // then
    assert_eq!(done.error, "");
    let jobs = jobs_of(&app, &linked, &pipeline).await;
    assert_eq!(jobs.len(), 2);
    let (new, old) = (&jobs[0], &jobs[1]);
    assert_eq!(text(&new.trigger_id), triggers[0]);
    let source = new.source.as_ref().unwrap();
    assert_eq!(source.event(), SourceEvent::PullRequest);
    assert_eq!(
        (source.sha.as_str(), source.branch.as_str()),
        (fixed, "feature/login")
    );
    assert!(!source.from_fork);
    let pulls: Vec<_> = source
        .pull_requests
        .iter()
        .map(|pull| (pull.number, pull.target_branch.as_str()))
        .collect();
    assert_eq!(pulls, vec![(42, "main")]);
    assert_eq!(old.source.as_ref().unwrap().sha, SHA);
    assert_eq!(old.status(), JobStatus::Pending);
}
