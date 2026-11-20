//! The tests of the read of the pipeline files at a commit: a file pipeline on another branch,
//! and the list of the files of a commit with their errors.

use scylla_domain::projects::{Project, ProjectId};
use scylla_proto::integrations::v1::PipelineSource;
use scylla_proto::pipelines::v1::git_ref::Kind;
use scylla_proto::pipelines::v1::PipelineFileFormat;
use scylla_testkit::{text, FakeProvider};
use sqlx::PgPool;
use tonic::Code;

use super::{
    app_with, assert_reason, by_key, linked, pipelines, BUILD, BUILD_PATH, DEPLOY, DEPLOY_PATH,
    FEATURE_SHA, LINT, LINT_PATH, MAIN_SHA,
};
use crate::assert_code;

/// A file of the default branch that names a secret that the project does not have.
const SECRET: &str = "version = 1\n\n[[nodes]]\nid = \"push\"\nexec = [\"make\"]\nenv = { TOKEN = { secret = \"registry\" } }\n";

/// A pull request adds a node to `build.toml`: the file of its branch has 5 nodes, and the
/// pipeline of the default branch still has 4 nodes.
#[sqlx::test(migrations = false)]
async fn get_file_pipeline_reads_the_file_of_another_branch(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    fake.github_files("heads/main", MAIN_SHA, &[(BUILD_PATH, BUILD)]);
    let bench =
        format!("{BUILD}\n[[nodes]]\nid = \"bench\"\nneeds = [\"test\"]\nexec = [\"make\"]\n");
    fake.github_files("heads/feature", FEATURE_SHA, &[(BUILD_PATH, &bench)]);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github").await;
    let (owner, project) = (linked.owner, linked.project.as_str());
    app.set_pipeline_source(owner, project, PipelineSource::File)
        .await
        .unwrap();
    let build = text(&by_key(&pipelines(&app, owner, project).await, "build").id).to_owned();

    // when
    let feature = Some(Kind::Branch("feature".to_owned()));
    let branch = app.get_file_pipeline(owner, &build, feature).await.unwrap();
    let main = app.get_file_pipeline(owner, &build, None).await.unwrap();

    // then
    assert_eq!(branch.nodes.len(), 5);
    let file = branch.file.as_ref().expect("a file");
    assert_eq!(
        (file.path.as_str(), file.commit_sha.as_str()),
        (BUILD_PATH, FEATURE_SHA)
    );
    let page = format!("https://github.com/octo/app/blob/{FEATURE_SHA}/{BUILD_PATH}");
    assert_eq!(file.url, page);
    assert_eq!(file.blob_sha.len(), 40);
    assert!(branch.content.contains("bench"));
    assert_eq!(branch.format(), PipelineFileFormat::Toml);
    assert_eq!(text(&branch.pipeline_id), build);
    assert_eq!(main.nodes.len(), 4);
    assert_eq!(
        app.get_pipeline(owner, &build).await.unwrap().nodes.len(),
        4
    );
}

/// An editor pipeline has no file, and a commit with no file of the key gives `NOT_FOUND`.
#[sqlx::test(migrations = false)]
async fn get_file_pipeline_needs_a_file_of_its_key(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    fake.github_files("heads/main", MAIN_SHA, &[(BUILD_PATH, BUILD)]);
    fake.github_files(FEATURE_SHA, FEATURE_SHA, &[(DEPLOY_PATH, DEPLOY)]);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github").await;
    let (owner, project) = (linked.owner, linked.project.as_str());
    app.set_pipeline_source(owner, project, PipelineSource::File)
        .await
        .unwrap();
    let build = text(&by_key(&pipelines(&app, owner, project).await, "build").id).to_owned();

    // when
    let editor = app.get_file_pipeline(owner, &linked.editor, None).await;
    let sha = Some(Kind::Sha(FEATURE_SHA.to_owned()));
    let missing = app.get_file_pipeline(owner, &build, sha).await;

    // then
    assert_reason(editor, "NOT_A_FILE_PIPELINE");
    assert_code(missing, Code::NotFound);
}

/// The list of the files of a commit gives each file with no text, its pipeline, its errors with
/// their lines, and its warnings. A refusal of the lookup is in the errors of the commit.
#[sqlx::test(migrations = false)]
async fn list_pipeline_files_gives_each_file_with_its_errors(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let files = [
        (BUILD_PATH, BUILD),
        (LINT_PATH, LINT),
        (".scylla/pipelines/push.toml", SECRET),
        (".scylla/pipelines/twice.toml", BUILD),
        (".scylla/pipelines/twice.yml", DEPLOY),
    ];
    fake.github_files("heads/main", MAIN_SHA, &files);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github").await;
    let viewer = app.user().await;
    let id = ProjectId::parse(&linked.project).unwrap();
    app.grant(&[Project::VIEWER.of(id, viewer)]).await;

    // when
    let listed = app.list_pipeline_files(viewer, &linked.project, None).await;

    // then
    let listed = listed.unwrap();
    assert_eq!(listed.commit_sha, MAIN_SHA);
    let keys: Vec<_> = listed.files.iter().map(|file| file.key.as_str()).collect();
    assert_eq!(keys, vec!["build", "lint", "push"]);
    assert!(listed.files.iter().all(|file| file.content.is_empty()));
    assert!(listed.files[0].errors.is_empty() && listed.files[0].pipeline_id.is_none());
    let lint = &listed.files[1].errors;
    assert_eq!(lint.len(), 1);
    assert!(lint[0].line > 0 && !lint[0].warning, "{lint:?}");
    assert_eq!(lint[0].field, "nodes[0].needs[0]");
    let warning = &listed.files[2].errors;
    assert!(warning.len() == 1 && warning[0].warning, "{warning:?}");
    assert_eq!(listed.errors.len(), 1);
    assert!(
        listed.errors[0].message.contains("twice"),
        "{:?}",
        listed.errors
    );
}

/// A project with no link has no files, and a stranger cannot list them.
#[sqlx::test(migrations = false)]
async fn list_pipeline_files_needs_a_link_and_a_reader(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let app = app_with(pool, &fake).await;
    let alone = linked(&app, "none").await;
    let linked = linked(&app, "github").await;
    let stranger = app.user().await;

    // when
    let unlinked = app
        .list_pipeline_files(alone.owner, &alone.project, None)
        .await;
    let refused = app
        .list_pipeline_files(stranger, &linked.project, None)
        .await;

    // then
    assert_reason(unlinked, "NO_REPOSITORY_LINK");
    assert_code(refused, Code::PermissionDenied);
}

/// A commit with no folder `.scylla` has no file, and a move to the files of this commit
/// archives the editor pipelines with no new pipeline.
#[sqlx::test(migrations = false)]
async fn a_commit_with_no_folder_has_no_file(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    fake.github_files("heads/main", MAIN_SHA, &[]);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github").await;
    let (owner, project) = (linked.owner, linked.project.as_str());

    // when
    let listed = app.list_pipeline_files(owner, project, None).await.unwrap();
    let moved = app.set_pipeline_source(owner, project, PipelineSource::File);

    // then
    assert!(listed.files.is_empty() && listed.errors.is_empty());
    moved.await.unwrap();
    let all = pipelines(&app, owner, project).await;
    assert_eq!(all.len(), 1);
    assert!(all[0].archived_at.is_some());
}
