//! The tests of the usages of a fragment: the pipelines that link it, in its reads and after its
//! delete.

use scylla_testkit::{exec_node, fragment_link, text, TestApp};
use sqlx::PgPool;
use tonic::Code;

use crate::{app, assert_code};

/// Makes a user with a project in its workspace, and returns the user, the workspace and the
/// project as text.
async fn owner_with_project(
    app: &TestApp,
) -> (scylla_domain::identity::actors::ActorId, String, String) {
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let project = app.create_project(owner, "web").await.unwrap();
    (owner, workspace, text(&project.id).to_owned())
}

/// A pipeline that links a fragment is a usage of it, with its version and its values.
#[sqlx::test(migrations = false)]
async fn get_fragment_shows_the_pipelines_that_link_it(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, workspace, project) = owner_with_project(&app).await;
    let fragment = text(
        &app.create_fragment(owner, &workspace, "build")
            .await
            .unwrap()
            .id,
    )
    .to_owned();
    let links = vec![fragment_link("build-block", &fragment, 1, &["build"])];
    let pipeline = app
        .create_linked_pipeline(owner, &project, vec![exec_node("build", &[])], links)
        .await
        .unwrap();

    // when
    let read = app.get_fragment(owner, &fragment).await.unwrap();

    // then
    assert_eq!(read.usages.len(), 1);
    let usage = &read.usages[0];
    assert_eq!(text(&usage.pipeline_id), text(&pipeline.id));
    assert_eq!(
        (usage.pipeline_name.as_str(), usage.project_name.as_str()),
        ("linked", "web")
    );
    assert_eq!(usage.version, 1);
    assert_eq!(usage.values.get("MODE").map(String::as_str), Some("debug"));
}

/// A delete removes the fragment from the library. The pipeline that links it keeps its nodes
/// and its link.
#[sqlx::test(migrations = false)]
async fn delete_fragment_keeps_the_pipelines_that_link_it(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, workspace, project) = owner_with_project(&app).await;
    let fragment = text(
        &app.create_fragment(owner, &workspace, "build")
            .await
            .unwrap()
            .id,
    )
    .to_owned();
    let links = vec![fragment_link("build-block", &fragment, 1, &["build"])];
    let pipeline = app
        .create_linked_pipeline(owner, &project, vec![exec_node("build", &[])], links)
        .await
        .unwrap();

    // when
    app.delete_fragment(owner, &fragment).await.unwrap();

    // then
    assert_code(
        app.get_fragment(owner, &fragment).await,
        Code::PermissionDenied,
    );
    assert!(app
        .list_fragments(owner, &workspace)
        .await
        .unwrap()
        .is_empty());
    let kept = app.get_pipeline(owner, text(&pipeline.id)).await.unwrap();
    assert_eq!(kept.nodes.len(), 1);
    assert_eq!(text(&kept.fragment_links[0].fragment_id), fragment);
    let saved = app
        .update_pipeline_links(owner, text(&pipeline.id), kept.version, None, None)
        .await;
    assert!(
        saved.is_ok(),
        "a pipeline with a link of a deleted fragment still saves"
    );
}
