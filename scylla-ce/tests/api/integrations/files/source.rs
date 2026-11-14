//! The tests of the source of the pipelines of a project: the move to the files of the default
//! branch, the move back to the editor, the sync of a removed file, and the read only pipelines.

use scylla_domain::projects::{Project, ProjectId};
use scylla_proto::integrations::v1::PipelineSource;
use scylla_proto::pipelines::v1::git_ref::Kind;
use scylla_proto::pipelines::v1::{PipelineLayout, Position};
use scylla_testkit::{exec_node, text, FakeProvider};
use sqlx::PgPool;
use tonic::Code;

use super::{
    app_with, assert_reason, by_key, linked, pipelines, BUILD, BUILD_PATH, DEPLOY, DEPLOY_PATH,
    LINT, LINT_PATH, MAIN_SHA, NEXT_SHA,
};
use crate::assert_code;

/// A move to the files reads the default branch: each file gives a file pipeline with its key,
/// its path, its nodes and its errors, and the editor pipelines become archived.
#[sqlx::test(migrations = false)]
async fn set_pipeline_source_moves_a_project_to_its_files(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let files = [
        (BUILD_PATH, BUILD),
        (DEPLOY_PATH, DEPLOY),
        (LINT_PATH, LINT),
    ];
    fake.github_files("heads/main", MAIN_SHA, &files);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github").await;

    // when
    let file = PipelineSource::File;
    let moved = app.set_pipeline_source(linked.owner, &linked.project, file);

    // then
    moved.await.unwrap();
    let all = pipelines(&app, linked.owner, &linked.project).await;
    assert_eq!(all.len(), 4);
    let editor = all
        .iter()
        .find(|pipeline| text(&pipeline.id) == linked.editor);
    let editor = editor.expect("the editor pipeline stays in the list");
    assert!(editor.archived_at.is_some());
    assert_eq!(editor.source(), PipelineSource::Editor);
    let build = by_key(&all, "build");
    assert_eq!(build.source(), PipelineSource::File);
    assert_eq!(
        (build.name.as_str(), build.file_path.as_str()),
        ("Build", BUILD_PATH)
    );
    assert_eq!(build.nodes.len(), 4);
    assert_eq!((build.file_error_count, build.archived_at), (0, None));
    assert_eq!(by_key(&all, "deploy").name, "Deploy");
    let lint = by_key(&all, "lint");
    assert_eq!((lint.file_error_count, lint.nodes.len()), (1, 0));
    assert_eq!(lint.name, "lint");
}

/// A project with no link cannot move to the files, and a viewer of the project cannot move it.
#[sqlx::test(migrations = false)]
async fn set_pipeline_source_needs_a_link_and_an_editor(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let app = app_with(pool, &fake).await;
    let alone = linked(&app, "none").await;
    let linked = linked(&app, "github").await;
    let viewer = app.user().await;
    let project = ProjectId::parse(&linked.project).unwrap();
    app.grant(&[Project::VIEWER.of(project, viewer)]).await;

    // when
    let file = PipelineSource::File;
    let unlinked = app
        .set_pipeline_source(alone.owner, &alone.project, file)
        .await;
    let refused = app.set_pipeline_source(viewer, &linked.project, file).await;
    let unspecified = PipelineSource::Unspecified;
    let empty = app.set_pipeline_source(linked.owner, &linked.project, unspecified);

    // then
    assert_reason(unlinked, "NO_REPOSITORY_LINK");
    assert_code(refused, Code::PermissionDenied);
    assert_code(empty.await, Code::InvalidArgument);
    let editor = app.get_pipeline(alone.owner, &alone.editor).await.unwrap();
    assert!(editor.archived_at.is_none());
}

/// A file pipeline and an archived pipeline are read only: a change and a delete give
/// `PIPELINE_READ_ONLY`, and a new pipeline in the project too. A save of the places works.
#[sqlx::test(migrations = false)]
async fn a_file_pipeline_and_an_archived_pipeline_are_read_only(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    fake.github_files("heads/main", MAIN_SHA, &[(BUILD_PATH, BUILD)]);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github").await;
    let (owner, project) = (linked.owner, linked.project.as_str());
    app.set_pipeline_source(owner, project, PipelineSource::File)
        .await
        .unwrap();
    let build = by_key(&pipelines(&app, owner, project).await, "build").clone();
    let build_id = text(&build.id).to_owned();
    let editor = app.get_pipeline(owner, &linked.editor).await.unwrap();

    // when
    let changed = app.update_pipeline(owner, &build_id, build.version, Some("New"), None);
    let changed = changed.await;
    let archived = app.update_pipeline(owner, &linked.editor, editor.version, Some("New"), None);
    let archived = archived.await;
    let deleted = app.delete_pipeline(owner, &build_id).await;
    let deleted_archived = app.delete_pipeline(owner, &linked.editor).await;
    let created = app.create_pipeline(owner, project, vec![exec_node("x", &[])]);
    let created = created.await;
    let layout = PipelineLayout {
        start: None,
        nodes: [("fetch".to_owned(), Position { x: 10, y: 20 })].into(),
    };
    let saved = app.save_pipeline_layout(owner, &build_id, Some(layout));

    // then
    assert_reason(changed, "PIPELINE_READ_ONLY");
    assert_reason(archived, "PIPELINE_READ_ONLY");
    assert_reason(deleted, "PIPELINE_READ_ONLY");
    assert_reason(deleted_archived, "PIPELINE_READ_ONLY");
    assert_reason(created, "PIPELINE_READ_ONLY");
    saved.await.unwrap();
    let layout = app.pipeline_layout(owner, &build_id).await.unwrap();
    assert_eq!(
        layout.expect("a layout").nodes["fetch"],
        Position { x: 10, y: 20 }
    );
}

/// A move back to the editor restores the archived pipelines and hides the file pipelines. Their
/// jobs stay, and a later move to the files shows them again.
#[sqlx::test(migrations = false)]
async fn set_pipeline_source_back_to_the_editor_restores_the_editor_pipelines(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    fake.github_files("heads/main", MAIN_SHA, &[(BUILD_PATH, BUILD)]);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github").await;
    let (owner, project) = (linked.owner, linked.project.as_str());
    app.set_pipeline_source(owner, project, PipelineSource::File)
        .await
        .unwrap();
    let build = by_key(&pipelines(&app, owner, project).await, "build").clone();
    let build_id = text(&build.id).to_owned();
    let job = app.run_pipeline(owner, &build_id).await.unwrap();

    // when
    let back = app.set_pipeline_source(owner, project, PipelineSource::Editor);
    back.await.unwrap();

    // then
    let all = pipelines(&app, owner, project).await;
    let ids: Vec<_> = all.iter().map(|pipeline| text(&pipeline.id)).collect();
    assert_eq!(ids, vec![linked.editor.as_str()]);
    assert!(all[0].archived_at.is_none());
    let hidden = app.get_pipeline(owner, &build_id).await.unwrap();
    assert!(hidden.archived_at.is_some());
    assert!(app.get_job(owner, text(&job.id)).await.is_ok());
    let again = app.set_pipeline_source(owner, project, PipelineSource::File);
    again.await.unwrap();
    let all = pipelines(&app, owner, project).await;
    assert_eq!(text(&by_key(&all, "build").id), build_id);
    assert!(by_key(&all, "build").archived_at.is_none());
}

/// A file that leaves the default branch marks its pipeline "Removed from main". Its jobs stay,
/// and a changed file gives a new version of its pipeline.
#[sqlx::test(migrations = false)]
async fn a_file_removed_from_the_default_branch_marks_its_pipeline(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let first = [(BUILD_PATH, BUILD), (DEPLOY_PATH, DEPLOY)];
    fake.github_files("heads/main", MAIN_SHA, &first);
    fake.github_files(MAIN_SHA, MAIN_SHA, &first);
    let renamed = BUILD.replace("name = \"Build\"", "name = \"Build all\"");
    fake.github_files("heads/main", NEXT_SHA, &[(BUILD_PATH, &renamed)]);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github").await;
    let (owner, project) = (linked.owner, linked.project.as_str());
    app.set_pipeline_source(owner, project, PipelineSource::File)
        .await
        .unwrap();
    let before = pipelines(&app, owner, project).await;
    let deploy = text(&by_key(&before, "deploy").id).to_owned();
    let first = Kind::Sha(MAIN_SHA.to_owned());
    let job = app.run_pipeline_on(owner, &deploy, first).await.unwrap();

    // when
    let synced = app.set_pipeline_source(owner, project, PipelineSource::File);
    synced.await.unwrap();

    // then
    let after = pipelines(&app, owner, project).await;
    let removed = by_key(&after, "deploy");
    assert!(removed.removed_at.is_some());
    assert!(by_key(&after, "build").removed_at.is_none());
    assert_eq!(by_key(&after, "build").name, "Build all");
    let version = |all: &[scylla_proto::pipelines::v1::Pipeline]| by_key(all, "build").version;
    assert_eq!(version(&after), version(&before) + 1);
    assert!(app.get_job(owner, text(&job.id)).await.is_ok());
}
