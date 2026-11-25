//! The tests of the editor pipelines next to the file pipelines of a project of files: the
//! creation, the keys that the two sources share, the mark of a key that a file takes, the report
//! that such a pipeline does not send, and the merge rules of the file pipelines only.

use scylla_proto::integrations::v1::PipelineSource;
use scylla_proto::pipelines::v1::git_ref::Kind;
use scylla_proto::pipelines::v1::{ListPipelineSummariesRequest, PipelineSummary};
use scylla_testkit::{exec_node, id, text, FakeProvider};
use sqlx::PgPool;
use tonic::Status;
use tonic_types::StatusExt;

use super::{
    app_with, assert_reason, by_key, files_project, linked, pipelines, BUILD, BUILD_PATH, DEPLOY,
    DEPLOY_PATH, MAIN_SHA, NEXT_SHA,
};

/// Checks that a call fails with `PIPELINE_KEY_TAKEN` and the key in the metadata.
fn assert_key_taken<T: std::fmt::Debug>(result: Result<T, Status>, key: &str) {
    let status = result.expect_err("the call must fail");
    let info = status.get_details_error_info().expect("ErrorInfo");
    assert_eq!(info.reason, "PIPELINE_KEY_TAKEN", "{status:?}");
    assert_eq!(info.metadata.get("key").map(String::as_str), Some(key));
}

/// `CreatePipeline` adds an editor pipeline to a project of files, next to its file pipelines.
#[sqlx::test(migrations = false)]
async fn create_pipeline_adds_an_editor_pipeline_to_a_project_of_files(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    fake.github_files("heads/main", MAIN_SHA, &[(DEPLOY_PATH, DEPLOY)]);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github").await;
    let (owner, project) = (linked.owner, linked.project.as_str());
    app.set_pipeline_source(owner, project, PipelineSource::File)
        .await
        .unwrap();

    // when
    let created = app.create_pipeline(owner, project, vec![exec_node("x", &[])]);
    let created = created.await.unwrap();

    // then
    assert_eq!(created.source(), PipelineSource::Editor);
    assert_eq!(created.key, "build");
    assert!(created.key_taken_at.is_none() && created.archived_at.is_none());
    let all = pipelines(&app, owner, project).await;
    assert_eq!(all.len(), 3);
    assert_eq!(by_key(&all, "deploy").source(), PipelineSource::File);
}

/// A creation or a new name with the key of a file of the default branch gives
/// `PIPELINE_KEY_TAKEN` with the key. A new name with the same key keeps the mark of the taken
/// key, and a new name with a free key frees the pipeline.
#[sqlx::test(migrations = false)]
async fn an_editor_pipeline_cannot_take_the_key_of_a_file(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let main = [(BUILD_PATH, BUILD), (DEPLOY_PATH, DEPLOY)];
    fake.github_files("heads/main", MAIN_SHA, &main);
    let app = app_with(pool, &fake).await;
    let files = files_project(&app, false).await;
    let (owner, project) = (files.linked.owner, files.linked.project.as_str());
    let editor = files.linked.editor.as_str();

    // when
    let created = app.create_pipeline(owner, project, vec![exec_node("x", &[])]);
    let created = created.await;
    let same = app.update_pipeline(owner, editor, 1, Some("Build"), None);
    let same = same.await.unwrap();
    let deploy = app.update_pipeline(owner, editor, same.version, Some("Deploy"), None);
    let deploy = deploy.await;
    let release = app.update_pipeline(owner, editor, same.version, Some("Release"), None);
    let release = release.await.unwrap();

    // then
    assert_key_taken(created, "build");
    assert!(same.key_taken_at.is_some());
    assert_key_taken(deploy, "deploy");
    assert_eq!(release.key, "release");
    assert!(release.key_taken_at.is_none());
}

/// A file that takes the key of an editor pipeline wins: the editor pipeline gets the mark, the
/// summaries tell it, and its job sends no report (`KEY_TAKEN`). When the file leaves the default
/// branch, the editor pipeline is free again.
#[sqlx::test(migrations = false)]
async fn a_file_that_takes_the_key_of_an_editor_pipeline_wins(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    fake.github_files("heads/main", MAIN_SHA, &[(BUILD_PATH, BUILD)]);
    fake.github_files(MAIN_SHA, MAIN_SHA, &[(BUILD_PATH, BUILD)]);
    fake.github_files("heads/main", NEXT_SHA, &[(DEPLOY_PATH, DEPLOY)]);
    let app = app_with(pool, &fake).await;
    let files = files_project(&app, true).await;
    let (owner, project) = (files.linked.owner, files.linked.project.as_str());
    let editor = files.linked.editor.as_str();

    // when
    let request = ListPipelineSummariesRequest {
        workspace_id: Some(id(files.linked.workspace.to_string())),
        ..ListPipelineSummariesRequest::default()
    };
    let summaries = app.list_pipeline_summaries(owner, request).await.unwrap();
    let job = app.run_pipeline_on(owner, editor, Kind::Sha(MAIN_SHA.to_owned()));
    let job = text(&job.await.unwrap().id).to_owned();
    let reports = app.list_job_reports(owner, &[&job]).await.unwrap().jobs;
    app.set_pipeline_source(owner, project, PipelineSource::File)
        .await
        .unwrap();
    let freed = app.get_pipeline(owner, editor).await.unwrap();

    // then
    let summary = |pipeline: &str| -> PipelineSummary {
        let found = summaries
            .pipelines
            .iter()
            .find(|summary| text(&summary.pipeline_id) == pipeline);
        found.expect("a summary").clone()
    };
    let taken = summary(editor);
    assert_eq!(taken.source(), PipelineSource::Editor);
    assert!(taken.key_taken);
    let build = summary(&files.build);
    assert_eq!(build.source(), PipelineSource::File);
    assert!(!build.key_taken);
    assert!(reports[0].reports.is_empty());
    assert_eq!(reports[0].no_report_reason, "KEY_TAKEN");
    assert!(freed.key_taken_at.is_none());
}

/// In a project of files, a merge rule cannot turn on for an editor pipeline
/// (`MERGE_BLOCK_FILE_ONLY`), and it can turn off. The list gives the source of each pipeline.
#[sqlx::test(migrations = false)]
async fn set_merge_block_needs_a_file_pipeline_in_a_project_of_files(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    fake.github_files("heads/main", MAIN_SHA, &[(BUILD_PATH, BUILD)]);
    let app = app_with(pool, &fake).await;
    let files = files_project(&app, false).await;
    let (owner, project) = (files.linked.owner, files.linked.project.as_str());
    let editor = files.linked.editor.as_str();

    // when
    let on = app
        .set_merge_block(owner, project, Some(editor), true)
        .await;
    let off = app
        .set_merge_block(owner, project, Some(editor), false)
        .await;
    let listed = app.list_merge_blocks(owner, project).await.unwrap();

    // then
    assert_reason(on, "MERGE_BLOCK_FILE_ONLY");
    assert!(!off.unwrap().enabled);
    let source = |pipeline: &str| {
        let block = listed
            .iter()
            .find(|block| text(&block.pipeline_id) == pipeline);
        block.expect("a rule").pipeline_source()
    };
    assert_eq!(source(editor), PipelineSource::Editor);
    assert_eq!(source(&files.build), PipelineSource::File);
}
