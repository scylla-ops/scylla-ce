//! The tests of the pipeline files of a GitLab project: the tree of `.scylla` and its blobs.

use scylla_proto::integrations::v1::PipelineSource;
use scylla_proto::pipelines::v1::git_ref::Kind;
use scylla_testkit::{text, FakeProvider};
use sqlx::PgPool;

use super::{app_with, by_key, linked, pipelines, BUILD, DEPLOY, FEATURE_SHA, MAIN_SHA};

/// A GitLab project with one file `.scylla/pipeline.yml` moves to the files: its pipeline has the
/// key `pipeline`, and the file of another branch reads with the token of the service account.
#[sqlx::test(migrations = false)]
async fn set_pipeline_source_reads_the_files_of_a_gitlab_project(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    fake.gitlab_files("main", MAIN_SHA, &[(".scylla/pipeline.yml", DEPLOY)]);
    fake.gitlab_files("feature", FEATURE_SHA, &[(".scylla/pipeline.toml", BUILD)]);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "gitlab").await;
    let (owner, project) = (linked.owner, linked.project.as_str());

    // when
    let moved = app.set_pipeline_source(owner, project, PipelineSource::File);
    moved.await.unwrap();
    let all = pipelines(&app, owner, project).await;
    let pipeline = text(&by_key(&all, "pipeline").id).to_owned();
    let feature = Some(Kind::Branch("feature".to_owned()));
    let branch = app.get_file_pipeline(owner, &pipeline, feature).await;

    // then
    let file = by_key(&all, "pipeline");
    assert_eq!(
        (file.name.as_str(), file.file_path.as_str()),
        ("Deploy", ".scylla/pipeline.yml")
    );
    assert_eq!(file.nodes.len(), 1);
    let branch = branch.unwrap();
    assert_eq!(branch.nodes.len(), 4);
    let page = format!("https://gitlab.com/group/app/-/blob/{FEATURE_SHA}/.scylla/pipeline.toml");
    assert_eq!(branch.file.expect("a file").url, page);
    let trees = fake.requests_to("GET", "/api/v4/projects/7/repository/tree");
    assert_eq!(trees.len(), 2);
    assert!(trees
        .iter()
        .all(|call| call.header("private-token") == Some("glpat-test")));
}
