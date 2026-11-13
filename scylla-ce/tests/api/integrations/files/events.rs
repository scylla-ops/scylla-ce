//! The tests of the runs of the Git events in a project of files: the files at the commit of the
//! event, their `on` tables, the sync of a push to the default branch, and the invalid files.

use scylla_core::infra::vcs::sign_github;
use scylla_proto::pipelines::v1::{JobStatus, SourceEvent};
use scylla_testkit::{
    text, wait_for_requests, DeliveryRow, FakeProvider, TestApp, TEST_APP_WEBHOOK_SECRET,
};
use serde_json::{json, Value};
use sqlx::PgPool;

use super::{
    app_with, bench, by_key, files_project, jobs_of, pipelines, BUILD, BUILD_PATH, DEPLOY,
    DEPLOY_PATH, FEATURE_SHA, LINT, LINT_PATH, MAIN_SHA, NEXT_SHA,
};

/// A commit of a tag.
const TAG_SHA: &str = "4444444444444444444444444444444444444444";

/// The path of the file of the pipeline `nightly`.
const NIGHTLY_PATH: &str = ".scylla/pipelines/nightly.toml";

/// The file of the pipeline `nightly`: one node and no `on` table.
const NIGHTLY: &str = "version = 1\n\n[[nodes]]\nid = \"nightly\"\nexec = [\"make\"]\n";

/// The path of the check runs of `octo/app`.
const CHECK_RUNS: &str = "/repos/octo/app/check-runs";

/// Posts a signed GitHub delivery of the App of the settings, checks that it gets 202, and waits
/// for its work.
async fn deliver(app: &TestApp, event: &str, key: &str, body: &Value) -> DeliveryRow {
    let body = body.to_string();
    let signature = sign_github(TEST_APP_WEBHOOK_SECRET.as_bytes(), body.as_bytes());
    let response = reqwest::Client::new()
        .post(format!("http://{}/integrations/github/webhook", app.addr))
        .header("x-hub-signature-256", signature)
        .header("x-github-event", event)
        .header("x-github-delivery", key)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 202);
    app.wait_for_delivery(key).await
}

/// Returns a GitHub push of a commit to a ref of `octo/app` (id 7), from the installation 42.
fn push(git_ref: &str, sha: &str) -> Value {
    json!({
        "ref": git_ref,
        "after": sha,
        "deleted": false,
        "head_commit": { "id": sha, "message": "Change the pipelines", "author": { "username": "octocat" } },
        "repository": { "id": 7, "full_name": "octo/app", "html_url": "https://github.com/octo/app" },
        "installation": { "id": 42 },
        "sender": { "login": "octocat", "avatar_url": null },
    })
}

/// Returns a GitHub pull request 12 of `octo/app` (id 7) that opens from its branch `feature`
/// into `main` at a commit, from the installation 42.
fn pull_opened(sha: &str) -> Value {
    let repo = json!({ "id": 7, "full_name": "octo/app" });
    json!({
        "action": "opened",
        "pull_request": {
            "number": 12,
            "title": "Add lint",
            "html_url": "https://github.com/octo/app/pull/12",
            "draft": false,
            "head": { "ref": "feature", "sha": sha, "repo": repo },
            "base": { "ref": "main", "sha": MAIN_SHA, "repo": repo },
            "user": { "login": "octocat", "avatar_url": null },
        },
        "repository": { "id": 7, "full_name": "octo/app", "html_url": "https://github.com/octo/app" },
        "installation": { "id": 42 },
        "sender": { "login": "octocat", "avatar_url": null },
    })
}

/// A push takes the files at its commit: each file pipeline whose `on` table matches the push
/// gets one run with the nodes of its file, as the runner of the triggers. A file with no `on`
/// table, a second event on the commit and a branch that no rule matches start nothing.
#[sqlx::test(migrations = false)]
async fn a_push_runs_the_file_pipelines_whose_on_table_matches(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let main = [(BUILD_PATH, BUILD), (DEPLOY_PATH, DEPLOY)];
    fake.github_files("heads/main", MAIN_SHA, &main);
    let release = [(BUILD_PATH, bench()), (DEPLOY_PATH, DEPLOY.to_owned())];
    let release = release
        .each_ref()
        .map(|(path, text)| (*path, text.as_str()));
    fake.github_files("heads/release/2", FEATURE_SHA, &release);
    fake.github_files("heads/feature/x", NEXT_SHA, &main);
    let app = app_with(pool, &fake).await;
    let files = files_project(&app, false).await;
    let (owner, project) = (files.linked.owner, files.linked.project.as_str());

    // when
    let release = push("refs/heads/release/2", FEATURE_SHA);
    let done = deliver(&app, "push", "d-1", &release).await;
    deliver(&app, "push", "d-2", &release).await;
    deliver(&app, "push", "d-3", &push("refs/heads/feature/x", NEXT_SHA)).await;

    // then
    assert_eq!(done.error, "");
    let jobs = jobs_of(&app, owner, &files.build).await;
    assert_eq!(jobs.len(), 1);
    let job = &jobs[0];
    assert_eq!((job.status(), job.nodes.len()), (JobStatus::Pending, 5));
    assert!(job.trigger_id.is_none());
    let runner = app.trigger_runner(files.linked.workspace).await.unwrap();
    assert_eq!(text(&job.created_by), runner);
    let source = job.source.as_ref().expect("a source");
    assert_eq!(source.event(), SourceEvent::Push);
    assert_eq!(
        (source.sha.as_str(), source.branch.as_str()),
        (FEATURE_SHA, "release/2")
    );
    let file = job.pipeline_file.as_ref().expect("a pipeline file");
    assert_eq!(
        (file.path.as_str(), file.commit_sha.as_str()),
        (BUILD_PATH, FEATURE_SHA)
    );
    let deploy = text(&by_key(&pipelines(&app, owner, project).await, "deploy").id).to_owned();
    assert!(jobs_of(&app, owner, &deploy).await.is_empty());
}

/// A push to the default branch syncs the file pipelines with its head: a changed file changes
/// its pipeline, a new file makes a pipeline, and a file that left the branch marks its pipeline
/// as removed. The run of the push has the nodes of the new file.
#[sqlx::test(migrations = false)]
async fn a_push_to_the_default_branch_syncs_the_file_pipelines(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let main = [(BUILD_PATH, BUILD), (DEPLOY_PATH, DEPLOY)];
    fake.github_files("heads/main", MAIN_SHA, &main);
    let bench = bench();
    let next = [(BUILD_PATH, bench.as_str()), (NIGHTLY_PATH, NIGHTLY)];
    fake.github_files("heads/main", NEXT_SHA, &next);
    let app = app_with(pool, &fake).await;
    let files = files_project(&app, false).await;
    let (owner, project) = (files.linked.owner, files.linked.project.as_str());

    // when
    let done = deliver(&app, "push", "d-1", &push("refs/heads/main", NEXT_SHA)).await;

    // then
    assert_eq!(done.error, "");
    let all = pipelines(&app, owner, project).await;
    let build = by_key(&all, "build");
    assert_eq!((build.nodes.len(), build.removed_at), (5, None));
    assert!(by_key(&all, "deploy").removed_at.is_some());
    assert!(by_key(&all, "nightly").removed_at.is_none());
    let jobs = jobs_of(&app, owner, &files.build).await;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].nodes.len(), 5);
    assert_eq!(jobs[0].source.as_ref().unwrap().sha, NEXT_SHA);
}

/// A pull request adds a file with a typo in `needs`. Its key gets a pipeline that is not on the
/// default branch, and a run that fails at once with no node and the errors of the file, so no
/// agent runs it. GitHub gets the check run "Invalid pipeline file" with the line of the error.
/// A tag does not run an invalid file.
#[sqlx::test(migrations = false)]
async fn an_invalid_file_fails_its_run_and_reports_its_line(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    fake.github_files("heads/main", MAIN_SHA, &[(BUILD_PATH, BUILD)]);
    let feature = [(BUILD_PATH, BUILD), (LINT_PATH, LINT)];
    fake.github_files("heads/feature", FEATURE_SHA, &feature);
    fake.github_files("tags/v1", TAG_SHA, &[(LINT_PATH, LINT)]);
    let check = json!({ "id": 99, "html_url": "https://github.com/octo/app/runs/99" });
    fake.respond("POST", CHECK_RUNS, 201, check.clone(), &[]);
    fake.respond("PATCH", &format!("{CHECK_RUNS}/99"), 200, check, &[]);
    let app = app_with(pool, &fake).await;
    let files = files_project(&app, true).await;
    let (owner, project) = (files.linked.owner, files.linked.project.as_str());

    // when
    let done = deliver(&app, "pull_request", "d-1", &pull_opened(FEATURE_SHA)).await;
    let created = wait_for_requests(&fake, "POST", CHECK_RUNS, 1).await;
    deliver(&app, "push", "d-2", &push("refs/tags/v1", TAG_SHA)).await;

    // then
    assert_eq!(done.error, "");
    let all = pipelines(&app, owner, project).await;
    let lint = by_key(&all, "lint");
    assert!(lint.removed_at.is_some());
    assert_eq!(lint.file_error_count, 1);
    let jobs = jobs_of(&app, owner, text(&lint.id)).await;
    assert_eq!(jobs.len(), 1);
    let job = &jobs[0];
    assert_eq!(job.status(), JobStatus::Failed);
    assert!(job.nodes.is_empty());
    assert_eq!(job.error, "the pipeline file is not valid");
    let error = &job.file_errors[0];
    assert_eq!(
        (error.path.as_str(), error.field.as_str()),
        (LINT_PATH, "nodes[0].needs[0]")
    );
    assert!(error.line > 0, "{error:?}");
    let file = job.pipeline_file.as_ref().expect("a pipeline file");
    assert_eq!(
        (file.path.as_str(), file.commit_sha.as_str()),
        (LINT_PATH, FEATURE_SHA)
    );
    assert!(jobs_of(&app, owner, &files.build).await.is_empty());
    let body = &created[0].body;
    assert_eq!(body["name"], "scylla/lint");
    assert_eq!(body["head_sha"], FEATURE_SHA);
    assert_eq!(
        (body["status"].as_str(), body["conclusion"].as_str()),
        (Some("completed"), Some("failure"))
    );
    assert_eq!(body["output"]["title"], "Invalid pipeline file");
    let summary = body["output"]["summary"].as_str().unwrap();
    let line = format!("{LINT_PATH}:{}:", error.line);
    assert!(summary.contains(&line), "{summary}");
}
