//! The tests of a job with a source at its dispatch and at its new run: the variables of the
//! commit, the secrets of a job from a fork, and the source of a new run.

use std::collections::HashMap;

use scylla_proto::agents::v1::Dispatch;
use scylla_proto::pipelines::v1::git_ref::Kind;
use scylla_proto::pipelines::v1::{env_var, ApprovalState, EnvVar, JobStatus, SourceEvent};
use scylla_testkit::{exec_node, text, FakeProvider};
use sqlx::PgPool;

use super::{app_with, github_commit, linked, SHA};

/// Another commit.
const OTHER_SHA: &str = "89abcdef0123456789abcdef0123456789abcdef";

/// Returns the variables of each node of a dispatch: the value and the mask of each name.
fn variables(dispatch: &Dispatch) -> Vec<HashMap<String, (String, bool)>> {
    dispatch
        .nodes
        .iter()
        .map(|node| {
            node.env
                .iter()
                .map(|var| (var.name.clone(), (var.value.clone(), var.masked)))
                .collect()
        })
        .collect()
}

/// Returns a variable that is not masked.
fn clear(value: &str) -> (String, bool) {
    (value.to_owned(), false)
}

/// Each node of a job on a commit gets the variables of the commit, not masked. A job with no
/// source gets none.
#[sqlx::test(migrations = false)]
async fn a_job_on_a_commit_gives_the_git_variables_to_each_node(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github_commit(&fake, "heads/main", SHA);
    let app = app_with(pool, &fake).await;
    let nodes = vec![exec_node("build", &[]), exec_node("test", &["build"])];
    let linked = linked(&app, "github", nodes).await;
    let main = Kind::Branch("main".to_owned());
    let job = app.run_pipeline_on(linked.owner, &linked.pipeline, main);
    let job = job.await.unwrap();
    let plain = super::linked(&app, "none", vec![exec_node("build", &[])]).await;
    app.run_pipeline(plain.owner, &plain.pipeline)
        .await
        .unwrap();

    // when
    let (dispatch, _up) = app.first_dispatch(linked.owner, linked.workspace).await;
    let (other, _other_up) = app.first_dispatch(plain.owner, plain.workspace).await;

    // then
    assert_eq!(dispatch.job_id.as_ref().unwrap().value, text(&job.id));
    let nodes = variables(&dispatch);
    assert_eq!(nodes.len(), 2);
    for vars in nodes {
        assert_eq!(vars["SCYLLA_GIT_SHA"], clear(SHA));
        assert_eq!(vars["SCYLLA_GIT_REF"], clear("refs/heads/main"));
        assert_eq!(vars["SCYLLA_GIT_BRANCH"], clear("main"));
        assert_eq!(vars["SCYLLA_GIT_TAG"], clear(""));
        assert_eq!(vars["SCYLLA_PR_NUMBER"], clear(""));
        assert_eq!(vars["SCYLLA_REPO"], clear("octo/app"));
        assert_eq!(
            vars["SCYLLA_REPO_URL"],
            clear("https://github.com/octo/app")
        );
        assert_eq!(vars["SCYLLA_EVENT"], clear("manual"));
        assert_eq!(vars["SCYLLA_GIT_FORK"], clear("false"));
    }
    let names: Vec<String> = variables(&other)
        .into_iter()
        .flat_map(|vars| vars.into_keys())
        .collect();
    assert!(
        names.iter().all(|name| !name.starts_with("SCYLLA_")),
        "{names:?}"
    );
}

/// A job from a fork gets an empty value for each secret, and `SCYLLA_GIT_FORK` is true. Its
/// literal variables do not change.
#[sqlx::test(migrations = false)]
async fn a_job_from_a_fork_gets_no_secret_value(pool: PgPool) {
    // given
    let app = crate::app(pool).await;
    let mut node = exec_node("build", &[]);
    let var = |name: &str, value| EnvVar {
        name: name.to_owned(),
        value: Some(value),
    };
    node.env = vec![
        var("TOKEN", env_var::Value::Secret("API_TOKEN".to_owned())),
        var("MODE", env_var::Value::Literal("ci".to_owned())),
    ];
    let linked = linked(&app, "github", vec![node]).await;
    let secret = app.create_secret(linked.owner, &linked.project, "API_TOKEN", "s3cret");
    secret.await.unwrap();
    let job = app
        .run_pipeline(linked.owner, &linked.pipeline)
        .await
        .unwrap();
    let job = text(&job.id).to_owned();
    app.insert_job_source(&job, SHA, "feature", "pull_request")
        .await;
    app.set_job_source(&job, "evil", Some(57), Some("evil/app"))
        .await;

    // when
    let (dispatch, _up) = app.first_dispatch(linked.owner, linked.workspace).await;
    let read = app.get_job(linked.owner, &job).await.unwrap();

    // then
    let vars = &variables(&dispatch)[0];
    assert_eq!(vars["TOKEN"], clear(""));
    assert_eq!(vars["MODE"], clear("ci"));
    assert_eq!(vars["SCYLLA_GIT_FORK"], clear("true"));
    assert_eq!(vars["SCYLLA_PR_NUMBER"], clear("57"));
    assert_eq!(vars["SCYLLA_EVENT"], clear("pull_request"));
    let source = read.source.expect("a source");
    assert!(source.from_fork);
    assert_eq!(source.fork_repository, "evil/app");
    assert_eq!(source.pull_requests[0].number, 57);
}

/// A new run keeps the source of the job, with its commit and its branch. A new run of a job
/// from a fork waits for an approval again.
#[sqlx::test(migrations = false)]
async fn rerun_job_keeps_the_source(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    github_commit(&fake, "heads/main", SHA);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github", vec![exec_node("build", &[])]).await;
    let (owner, pipeline) = (linked.owner, linked.pipeline.as_str());
    let main = Kind::Branch("main".to_owned());
    let job = app.run_pipeline_on(owner, pipeline, main).await.unwrap();
    let job = text(&job.id).to_owned();
    app.cancel_job(owner, &job).await.unwrap();
    let fork = app.run_pipeline(owner, pipeline).await.unwrap();
    let fork = text(&fork.id).to_owned();
    app.insert_job_source(&fork, OTHER_SHA, "feature", "pull_request")
        .await;
    app.set_job_source(&fork, "evil", Some(57), Some("evil/app"))
        .await;
    app.cancel_job(owner, &fork).await.unwrap();

    // when
    let again = app.rerun_job(owner, &job).await.unwrap();
    let fork_again = app.rerun_job(owner, &fork).await.unwrap();

    // then
    assert_eq!(again.status, JobStatus::Pending as i32);
    assert_eq!(text(&again.rerun_of), job);
    let source = again.source.expect("a source");
    assert_eq!((source.sha.as_str(), source.branch.as_str()), (SHA, "main"));
    assert_eq!(source.event, SourceEvent::Manual as i32);
    assert_eq!(source.author_login, "octocat");
    assert_eq!(fork_again.status, JobStatus::AwaitingApproval as i32);
    let approval = fork_again.approval.expect("an approval");
    assert_eq!(approval.state, ApprovalState::Waiting as i32);
    assert!(approval.decided_by.is_none() && approval.changes_pipeline_files);
    let changes = "https://github.com/octo/app/pull/57/files";
    assert_eq!(approval.changes_url, changes);
    let source = fork_again.source.expect("a source");
    assert_eq!(
        (source.sha.as_str(), source.event),
        (OTHER_SHA, SourceEvent::PullRequest as i32)
    );
}
