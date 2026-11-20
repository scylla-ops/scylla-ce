//! The tests of the rules of a project whose pipelines move between the editor and the files of
//! its repository.

use scylla_proto::integrations::v1::PipelineSource;
use scylla_testkit::{exec_node, text, TestApp};
use sqlx::PgPool;

use super::{app_with_fake, linked, Linked};

/// The head of the default branch.
const MAIN_SHA: &str = "1111111111111111111111111111111111111111";

/// The file of the pipeline `deploy`.
const DEPLOY: &str = "version: 1\nname: Deploy\nnodes:\n  - id: ship\n    exec: [make, ship]\n";

/// Returns the contexts of the rules of the list of the owner, in name order.
async fn contexts(app: &TestApp, linked: &Linked) -> Vec<String> {
    let listed = app.list_merge_blocks(linked.owner, &linked.project).await;
    let mut contexts: Vec<_> = listed
        .unwrap()
        .into_iter()
        .map(|rule| rule.context)
        .collect();
    contexts.sort();
    contexts
}

/// A list hides an archived pipeline, except a pipeline whose rule must be on. A move to the
/// files archives the editor pipelines, and a move back archives the file pipelines.
#[sqlx::test(migrations = false)]
async fn list_merge_blocks_hides_an_archived_pipeline(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    let (owner, project) = (linked.owner, linked.project.as_str());
    let lint = app.create_pipeline(owner, project, vec![exec_node("lint", &[])]);
    let lint = text(&lint.await.unwrap().id).to_owned();
    app.update_pipeline(owner, &lint, 1, Some("Lint"), None)
        .await
        .unwrap();
    fake.answer_github_rules_token(42);
    fake.answer_github_rulesets("octo/app", 99);
    fake.answer_github_ruleset("octo/app", 99, "active", &["scylla/build"]);
    let build = Some(linked.pipeline.as_str());
    app.set_merge_block(owner, project, build, true)
        .await
        .unwrap();
    let files = [(".scylla/pipelines/deploy.yaml", DEPLOY)];
    fake.github_files("heads/main", MAIN_SHA, &files);

    // when
    app.set_pipeline_source(owner, project, PipelineSource::File)
        .await
        .unwrap();
    let of_files = contexts(&app, &linked).await;
    app.set_pipeline_source(owner, project, PipelineSource::Editor)
        .await
        .unwrap();
    let of_editor = contexts(&app, &linked).await;

    // then
    assert_eq!(of_files, ["scylla/build", "scylla/deploy"]);
    assert_eq!(of_editor, ["scylla/build", "scylla/lint"]);
}
