//! The tests of the places of the nodes in the creation, the change and the delete of a pipeline.

use scylla_proto::pipelines::v1::{PipelineLayout, Position};
use scylla_testkit::{exec_node, text};
use sqlx::PgPool;
use tonic::Code;

use super::pipeline_layouts::{layout, pipeline, rows};
use crate::{app, assert_code, assert_invalid_field};

/// `CreatePipeline` saves the places of its nodes, and a bad place stops the creation.
#[sqlx::test(migrations = false)]
async fn create_pipeline_saves_the_places(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let project = text(&project.id).to_owned();
    let nodes = || vec![exec_node("build", &[])];
    let places = PipelineLayout {
        start: Some(Position { x: 1, y: 2 }),
        ..layout(&[("build", 3, 4), ("gone", 5, 6)])
    };

    // when
    let created = app
        .create_pipeline_with_layout(owner, &project, nodes(), places)
        .await
        .unwrap();
    let bad = layout(&[("build", 1_000_001, 0)]);
    let bad = app
        .create_pipeline_with_layout(owner, &project, nodes(), bad)
        .await;

    // then
    let id = text(&created.id).to_owned();
    let saved = app.pipeline_layout(owner, &id).await.unwrap().unwrap();
    assert_eq!(saved.start, Some(Position { x: 1, y: 2 }));
    assert_eq!(saved.nodes, layout(&[("build", 3, 4)]).nodes);
    assert_invalid_field(&bad.unwrap_err(), "layout.nodes[build]");
    let listed = app.list_project_pipelines(owner, &project, 10, "").await;
    assert_eq!(listed.unwrap().pipelines.len(), 1);
}

/// `UpdatePipeline` removes the places of the nodes that are gone, keeps the other places, and
/// saves the places of the new nodes.
#[sqlx::test(migrations = false)]
async fn update_pipeline_removes_the_places_of_gone_nodes(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _, created) = pipeline(&app).await;
    let id = text(&created.id).to_owned();
    let places = layout(&[("build", 1, 2), ("test", 3, 4)]);
    app.save_pipeline_layout(owner, &id, Some(places))
        .await
        .unwrap();
    let nodes = vec![exec_node("build", &[]), exec_node("deploy", &["build"])];

    // when
    let updated = app
        .update_pipeline_with_layout(
            owner,
            &id,
            created.version,
            Some(nodes),
            layout(&[("deploy", 5, 6), ("test", 7, 8)]),
        )
        .await
        .unwrap();

    // then
    assert_eq!(updated.version, created.version + 1);
    let saved = app.pipeline_layout(owner, &id).await.unwrap().unwrap();
    assert_eq!(
        saved.nodes,
        layout(&[("build", 1, 2), ("deploy", 5, 6)]).nodes
    );
}

/// When the last place goes, a read gives no layout. A change of the name keeps the places.
#[sqlx::test(migrations = false)]
async fn update_pipeline_keeps_the_places_with_no_new_nodes(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _, created) = pipeline(&app).await;
    let id = text(&created.id).to_owned();
    let place = layout(&[("test", 3, 4)]);
    app.save_pipeline_layout(owner, &id, Some(place.clone()))
        .await
        .unwrap();

    // when
    let renamed = app
        .update_pipeline(owner, &id, created.version, Some("new name"), None)
        .await
        .unwrap();
    let after_rename = app.pipeline_layout(owner, &id).await.unwrap();
    let nodes = vec![exec_node("build", &[])];
    app.update_pipeline(owner, &id, renamed.version, None, Some(nodes))
        .await
        .unwrap();
    let after_removal = app.pipeline_layout(owner, &id).await.unwrap();

    // then
    assert_eq!(after_rename.unwrap().nodes, place.nodes);
    assert_eq!(after_removal, None);
}

/// A change with another version saves no place.
#[sqlx::test(migrations = false)]
async fn update_pipeline_with_a_stale_version_saves_no_place(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _, created) = pipeline(&app).await;
    let id = text(&created.id).to_owned();

    // when
    let stale = app
        .update_pipeline_with_layout(
            owner,
            &id,
            created.version + 1,
            None,
            layout(&[("build", 1, 2)]),
        )
        .await;

    // then
    assert_code(stale, Code::Aborted);
    assert_eq!(app.pipeline_layout(owner, &id).await.unwrap(), None);
}

/// The places go with their pipeline, and with the project of their pipeline.
#[sqlx::test(migrations = false)]
async fn a_delete_removes_the_places(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project, first) = pipeline(&app).await;
    let first = text(&first.id).to_owned();
    let nodes = vec![exec_node("build", &[])];
    let second = app.create_pipeline(owner, &project, nodes).await.unwrap();
    let second = text(&second.id).to_owned();
    for id in [&first, &second] {
        let place = Some(layout(&[("build", 1, 2)]));
        app.save_pipeline_layout(owner, id, place).await.unwrap();
    }

    // when
    let (first_before, second_before) = (rows(&app, &first).await, rows(&app, &second).await);
    app.delete_pipeline(owner, &first).await.unwrap();
    let first_after = rows(&app, &first).await;
    app.delete_project(owner, &project).await.unwrap();

    // then
    assert_eq!((first_before, second_before), (1, 1));
    assert_eq!(first_after, 0);
    assert_eq!(rows(&app, &second).await, 0);
}
