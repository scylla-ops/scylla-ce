//! The tests of the tables of the Git integration that no route shows yet: the fixtures fit the
//! tables, one automatic job for each pipeline and commit, and the approval of a job from a fork.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_testkit::{exec_node, text, LinkRow, TestApp};
use serde_json::json;
use sqlx::PgPool;

use crate::app;

/// A commit: 40 hexadecimal characters.
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

/// Makes a linked project with one pipeline, and returns its owner and the pipeline.
async fn linked_pipeline(app: &TestApp) -> (ActorId, String) {
    let owner = app.user().await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let workspace = WorkspaceId::parse(text(&project.workspace_id)).unwrap();
    let connection = app
        .insert_github_connection(workspace, owner, 42, "octo")
        .await;
    let link = LinkRow {
        project: text(&project.id),
        connection: &connection,
        external_id: "7",
        full_name: "octo/app",
        default_branch: "main",
    };
    app.insert_repository_link(&link).await;
    let pipeline = app
        .create_pipeline(owner, text(&project.id), vec![exec_node("build", &[])])
        .await
        .unwrap();
    (owner, text(&pipeline.id).to_owned())
}

/// Returns the number of rows of a query that counts.
async fn count(app: &TestApp, sql: &'static str, id: &str) -> i64 {
    sqlx::query_scalar(sql)
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .expect("count")
}

/// The fixtures fit the tables, and a second automatic job of a pipeline on the same commit is
/// refused. A manual job on the same commit is not.
#[sqlx::test(migrations = false)]
async fn a_pipeline_has_one_automatic_job_for_each_commit(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, pipeline) = linked_pipeline(&app).await;
    let first = app.run_pipeline(owner, &pipeline).await.unwrap();
    let (second, manual) = (
        app.run_pipeline(owner, &pipeline).await.unwrap(),
        app.run_pipeline(owner, &pipeline).await.unwrap(),
    );
    let trigger = app
        .insert_git_trigger(
            &pipeline,
            "on push",
            "push",
            json!({ "branches": ["main"] }),
        )
        .await;
    app.insert_job_source(text(&first.id), SHA, "main", "push")
        .await;
    app.insert_job_source(text(&manual.id), SHA, "main", "manual")
        .await;

    // when
    let again = sqlx::query(
        "INSERT INTO job_sources (job_id, pipeline_id, provider, repository, url, sha, event,
                                  automatic)
         SELECT id, pipeline_id, 'github', 'octo/app', 'https://github.com/octo/app', $2,
                'pull_request', true
         FROM jobs WHERE id = $1::uuid",
    )
    .bind(text(&second.id))
    .bind(SHA)
    .execute(&app.pool)
    .await;

    // then
    let sources = "SELECT count(*) FROM job_sources WHERE pipeline_id = $1::uuid";
    let triggers = "SELECT count(*) FROM triggers WHERE id = $1::uuid AND rules IS NOT NULL";
    assert!(again.is_err(), "a second automatic job on one commit");
    assert_eq!(count(&app, sources, &pipeline).await, 2);
    assert_eq!(count(&app, triggers, &trigger).await, 1);
}

/// Runs a change of a job, with the id of the job as `$1`, and tells if the tables accept it.
async fn change(app: &TestApp, job: &str, sql: &'static str) -> bool {
    sqlx::query(sql).bind(job).execute(&app.pool).await.is_ok()
}

/// A job that waits for an approval has the approval `waiting`, and an approved job has its
/// approver and the time.
#[sqlx::test(migrations = false)]
async fn a_job_from_a_fork_waits_for_its_approval(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, pipeline) = linked_pipeline(&app).await;
    let job = app.run_pipeline(owner, &pipeline).await.unwrap();
    let id = text(&job.id);

    // when
    let no_approval = change(
        &app,
        id,
        "UPDATE jobs SET status = 'awaiting_approval' WHERE id = $1::uuid",
    )
    .await;
    let waiting = change(
        &app,
        id,
        "UPDATE jobs SET status = 'awaiting_approval', approval = 'waiting' WHERE id = $1::uuid",
    )
    .await;
    let no_approver = change(
        &app,
        id,
        "UPDATE jobs SET status = 'pending', approval = 'approved' WHERE id = $1::uuid",
    )
    .await;
    let approved = change(
        &app,
        id,
        "UPDATE jobs SET status = 'pending', approval = 'approved', approved_by = created_by,
                         approved_at = now()
         WHERE id = $1::uuid",
    )
    .await;

    // then
    assert!(!no_approval, "a waiting job needs the approval");
    assert!(waiting, "a waiting job with the approval");
    assert!(!no_approver, "an approval needs its approver");
    assert!(approved, "an approved job with its approver");
}
