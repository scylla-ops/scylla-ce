//! The tests of a run on a branch, a tag or a SHA of the linked repository.

use scylla_core::api::ServeOptions;
use scylla_proto::integrations::v1::VcsProvider;
use scylla_proto::pipelines::v1::git_ref::Kind;
use scylla_proto::pipelines::v1::{JobStatus, SourceEvent};
use scylla_testkit::{exec_node, text, FakeProvider, TestApp};
use serde_json::json;
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

use super::{app_with, github_commit, linked, GITLAB_TOKEN, SHA};
use crate::{assert_code, assert_invalid_field};

/// Returns the reason of a refusal.
fn reason(status: &Status) -> String {
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    status.get_details_error_info().expect("ErrorInfo").reason
}

/// A run on a branch of a GitHub repository gets the commit of its head, with a token narrowed
/// to the repository: the SHA, the ref, the author and the title.
#[sqlx::test(migrations = false)]
async fn run_pipeline_on_a_github_branch_gives_the_commit(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github_commit(&fake, "heads/main", SHA);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github", vec![exec_node("build", &[])]).await;
    let main = Kind::Branch("main".to_owned());

    // when
    let job = app.run_pipeline_on(linked.owner, &linked.pipeline, main);
    let job = job.await.unwrap();
    let read = app.get_job(linked.owner, text(&job.id)).await.unwrap();

    // then
    assert_eq!(job.status, JobStatus::Pending as i32);
    let source = read.source.expect("a source");
    assert_eq!(source.provider, VcsProvider::Github as i32);
    assert_eq!(
        (source.repository.as_str(), source.sha.as_str()),
        ("octo/app", SHA)
    );
    assert_eq!(source.repository_url, "https://github.com/octo/app");
    let commit = format!("https://github.com/octo/app/commit/{SHA}");
    assert_eq!(source.commit_url, commit);
    let refs = (
        source.r#ref.as_str(),
        source.branch.as_str(),
        source.tag.as_str(),
    );
    assert_eq!(refs, ("refs/heads/main", "main", ""));
    assert_eq!(source.author_login, "octocat");
    assert_eq!(source.author_avatar_url, "https://avatars.example/u/1");
    assert_eq!(source.message_title, "Fix the build");
    assert_eq!(source.event, SourceEvent::Manual as i32);
    assert!(!source.from_fork && source.pull_requests.is_empty() && read.approval.is_none());
    let tokens = fake.requests_to("POST", "/app/installations/42/access_tokens");
    let narrow = json!({ "repository_ids": [7], "permissions": { "contents": "read" } });
    assert_eq!(tokens[0].body, narrow);
    let reads = fake.requests_to("GET", "/repos/octo/app/commits/heads/main");
    assert_eq!(reads[0].header("authorization"), Some("Bearer ghs_test"));
}

/// A run on a tag of a GitLab project gets its commit with the token of the service account.
/// GitLab gives no login of the author.
#[sqlx::test(migrations = false)]
async fn run_pipeline_on_a_gitlab_tag_gives_the_commit(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let path = "/api/v4/projects/7/repository/commits/v1.2.0";
    let commit = json!({ "id": SHA, "message": "Release 1.2.0", "author_name": "Lea" });
    fake.respond("GET", path, 200, commit, &[]);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "gitlab", vec![exec_node("build", &[])]).await;
    let tag = Kind::Tag("v1.2.0".to_owned());

    // when
    let job = app.run_pipeline_on(linked.owner, &linked.pipeline, tag);
    let job = job.await.unwrap();

    // then
    let source = job.source.expect("a source");
    assert_eq!(source.provider, VcsProvider::Gitlab as i32);
    assert_eq!(source.repository_url, "https://gitlab.com/group/app");
    let commit = format!("https://gitlab.com/group/app/-/commit/{SHA}");
    assert_eq!(source.commit_url, commit);
    let refs = (
        source.r#ref.as_str(),
        source.branch.as_str(),
        source.tag.as_str(),
    );
    assert_eq!(refs, ("refs/tags/v1.2.0", "", "v1.2.0"));
    assert_eq!(
        (source.sha.as_str(), source.author_login.as_str()),
        (SHA, "")
    );
    assert_eq!(source.message_title, "Release 1.2.0");
    let reads = fake.requests_to("GET", path);
    assert_eq!(reads[0].header("private-token"), Some(GITLAB_TOKEN));
}

/// A run on a short SHA gets the full commit with no ref. A branch with a slash keeps the slash
/// in the route, and the second run uses the token of the cache.
#[sqlx::test(migrations = false)]
async fn run_pipeline_on_a_sha_gives_the_full_commit(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github_commit(&fake, "0123456", SHA);
    github_commit(&fake, "heads/release/1.2", SHA);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github", vec![exec_node("build", &[])]).await;
    let (owner, pipeline) = (linked.owner, linked.pipeline.as_str());

    // when
    let short = app.run_pipeline_on(owner, pipeline, Kind::Sha("0123456".to_owned()));
    let short = short.await.unwrap().source.expect("a source");
    let release = Kind::Branch("release/1.2".to_owned());
    let release = app.run_pipeline_on(owner, pipeline, release).await.unwrap();
    let release = release.source.expect("a source");

    // then
    assert_eq!(short.sha, SHA);
    assert!(short.r#ref.is_empty() && short.branch.is_empty() && short.tag.is_empty());
    assert_eq!(release.branch, "release/1.2");
    assert_eq!(release.r#ref, "refs/heads/release/1.2");
    let tokens = fake.requests_to("POST", "/app/installations/42/access_tokens");
    assert_eq!(tokens.len(), 1);
}

/// A run on a ref refuses a project with no link, a ref that is not valid, a ref that the
/// provider does not know, and a connection that is not active. No job starts.
#[sqlx::test(migrations = false)]
async fn run_pipeline_on_a_ref_refuses_what_cannot_run(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github_commit(&fake, "heads/main", SHA);
    let app = app_with(pool, &fake).await;
    let alone = linked(&app, "none", vec![exec_node("build", &[])]).await;
    let github = linked(&app, "github", vec![exec_node("build", &[])]).await;
    let (owner, pipeline) = (github.owner, github.pipeline.as_str());
    let main = || Kind::Branch("main".to_owned());

    // when
    let no_link = app
        .run_pipeline_on(alone.owner, &alone.pipeline, main())
        .await;
    let bad_sha = app.run_pipeline_on(owner, pipeline, Kind::Sha("xyz".to_owned()));
    let bad_sha = bad_sha.await;
    let bad_branch = app.run_pipeline_on(owner, pipeline, Kind::Branch("a b".to_owned()));
    let bad_branch = bad_branch.await;
    let gone = app.run_pipeline_on(owner, pipeline, Kind::Branch("gone".to_owned()));
    let gone = gone.await;
    sqlx::query("UPDATE vcs_connections SET state = 'suspended', state_reason = 'APP_SUSPENDED'")
        .execute(&app.pool)
        .await
        .unwrap();
    let suspended = app.run_pipeline_on(owner, pipeline, main()).await;

    // then
    assert_eq!(reason(&no_link.unwrap_err()), "NO_REPOSITORY_LINK");
    assert_invalid_field(&bad_sha.unwrap_err(), "ref.sha");
    assert_invalid_field(&bad_branch.unwrap_err(), "ref.branch");
    assert_code(gone, Code::NotFound);
    let suspended = suspended.unwrap_err();
    assert_eq!(reason(&suspended), "REPOSITORY_NOT_ACTIVE");
    let info = suspended.get_details_error_info().unwrap();
    assert_eq!(
        info.metadata.get("reason").map(String::as_str),
        Some("APP_SUSPENDED")
    );
    let listed = app
        .list_pipeline_jobs(owner, pipeline, 0, "")
        .await
        .unwrap();
    assert!(listed.jobs.is_empty());
}

/// A connection of the GitHub App of the instance runs on a GitHub ref with the keys of the row
/// of the App, on a server with no App in its settings.
#[sqlx::test(migrations = false)]
async fn run_pipeline_on_a_github_ref_uses_the_app_of_the_instance(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github_commit(&fake, "heads/main", SHA);
    let options = ServeOptions {
        vcs: fake.options(false),
        ..ServeOptions::default()
    };
    let app = TestApp::spawn_with(pool, scylla_ce::edition(), options).await;
    let linked = linked(&app, "github", vec![exec_node("build", &[])]).await;
    let connection: String = sqlx::query_scalar(
        "SELECT connection_id::text FROM repository_links WHERE project_id = $1::uuid",
    )
    .bind(&linked.project)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    app.give_instance_app(&connection, linked.owner).await;
    let main = Kind::Branch("main".to_owned());

    // when
    let job = app
        .run_pipeline_on(linked.owner, &linked.pipeline, main)
        .await;

    // then
    let source = job.unwrap().source.expect("a source");
    assert_eq!(source.sha, SHA);
    let tokens = fake.requests_to("POST", "/app/installations/42/access_tokens");
    assert_eq!(tokens.len(), 1);
    let bearer = tokens[0].header("authorization").unwrap_or_default();
    assert!(bearer.starts_with("Bearer "), "{bearer}");
}

/// A run on a GitHub ref on a server with no GitHub App gives `APP_MISSING`, and a run with no
/// ref still works.
#[sqlx::test(migrations = false)]
async fn run_pipeline_on_a_github_ref_needs_the_app(pool: PgPool) {
    // given
    let app = crate::app(pool).await;
    let linked = linked(&app, "github", vec![exec_node("build", &[])]).await;
    let main = Kind::Branch("main".to_owned());

    // when
    let refused = app
        .run_pipeline_on(linked.owner, &linked.pipeline, main)
        .await;
    let plain = app.run_pipeline(linked.owner, &linked.pipeline).await;

    // then
    assert_eq!(reason(&refused.unwrap_err()), "APP_MISSING");
    assert!(plain.unwrap().source.is_none());
}
