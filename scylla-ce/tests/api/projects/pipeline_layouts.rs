//! The tests of `SavePipelineLayout`: the merge of the places, their limits and the roles.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_proto::pipelines::v1::{Pipeline, PipelineLayout, Position};
use scylla_testkit::{exec_node, text, TestApp};
use sqlx::PgPool;
use tonic::Code;

use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// Creates a user, a project and a pipeline with the nodes `build` and `test`. Returns the user,
/// the project and the pipeline.
pub(super) async fn pipeline(app: &TestApp) -> (ActorId, String, Pipeline) {
    let owner = app.user().await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let project = text(&project.id).to_owned();
    let nodes = vec![exec_node("build", &[]), exec_node("test", &["build"])];
    let created = app.create_pipeline(owner, &project, nodes).await.unwrap();
    (owner, project, created)
}

/// Makes a layout from places, with no start.
pub(super) fn layout(places: &[(&str, i32, i32)]) -> PipelineLayout {
    PipelineLayout {
        start: None,
        nodes: places
            .iter()
            .map(|(id, x, y)| ((*id).to_owned(), Position { x: *x, y: *y }))
            .collect(),
    }
}

/// Returns the number of rows of `pipeline_layouts` for a pipeline.
pub(super) async fn rows(app: &TestApp, id: &str) -> i64 {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM pipeline_layouts WHERE pipeline_id = $1"#,
        PipelineId::parse(id).unwrap().to_db()
    )
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

/// A new pipeline has no layout. A save holds only the moved nodes, and the other places stay. A
/// save does not change the version or the time of the pipeline.
#[sqlx::test(migrations = false)]
async fn save_pipeline_layout_keeps_the_other_places(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _, created) = pipeline(&app).await;
    let id = text(&created.id).to_owned();
    let before = app.pipeline_layout(owner, &id).await.unwrap();
    let start = Position { x: -5, y: 7 };
    let first = PipelineLayout {
        start: Some(start),
        ..layout(&[("build", 10, -20)])
    };

    // when
    let saves = [
        first,
        layout(&[("test", 30, 40)]),
        layout(&[("build", 11, 21)]),
    ];
    for save in saves {
        app.save_pipeline_layout(owner, &id, Some(save))
            .await
            .unwrap();
    }

    // then
    assert_eq!(before, None);
    let saved = app.pipeline_layout(owner, &id).await.unwrap().unwrap();
    assert_eq!(saved.start, Some(start));
    let expected = layout(&[("build", 11, 21), ("test", 30, 40)]);
    assert_eq!(saved.nodes, expected.nodes);
    let read = app.get_pipeline(owner, &id).await.unwrap();
    assert_eq!(read.version, created.version);
    assert_eq!(read.updated_at, created.updated_at);
}

/// Two saves at one time both stay.
#[sqlx::test(migrations = false)]
async fn save_pipeline_layout_merges_two_saves_at_one_time(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _, created) = pipeline(&app).await;
    let id = text(&created.id).to_owned();

    // when
    let (first, second) = tokio::join!(
        app.save_pipeline_layout(owner, &id, Some(layout(&[("build", 1, 2)]))),
        app.save_pipeline_layout(owner, &id, Some(layout(&[("test", 3, 4)]))),
    );

    // then
    assert!(first.is_ok() && second.is_ok());
    let saved = app.pipeline_layout(owner, &id).await.unwrap().unwrap();
    assert_eq!(
        saved.nodes,
        layout(&[("build", 1, 2), ("test", 3, 4)]).nodes
    );
}

/// An id that is not a node of the pipeline is ignored, and a save of such ids alone adds no row.
#[sqlx::test(migrations = false)]
async fn save_pipeline_layout_ignores_an_id_that_is_not_a_node(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _, created) = pipeline(&app).await;
    let id = text(&created.id).to_owned();
    let unknown = layout(&[("gone", 3, 4), ("NOT AN ID", 5, 6)]);
    let mixed = layout(&[("build", 1, 2), ("gone", 3, 4), ("NOT AN ID", 5, 6)]);

    // when
    let alone = app.save_pipeline_layout(owner, &id, Some(unknown)).await;
    let rows_after_alone = rows(&app, &id).await;
    let with_a_node = app.save_pipeline_layout(owner, &id, Some(mixed)).await;

    // then
    assert!(alone.is_ok() && with_a_node.is_ok());
    assert_eq!(rows_after_alone, 0);
    let saved = app.pipeline_layout(owner, &id).await.unwrap().unwrap();
    assert_eq!(saved.nodes, layout(&[("build", 1, 2)]).nodes);
}

/// A request needs a layout, a place is -1000000 to 1000000, and a request has at most 256 places
/// of nodes.
#[sqlx::test(migrations = false)]
async fn save_pipeline_layout_checks_the_places(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _, created) = pipeline(&app).await;
    let id = text(&created.id).to_owned();
    let names: Vec<String> = (0..257).map(|index| format!("n{index}")).collect();
    let places = |count: usize| {
        let places: Vec<(&str, i32, i32)> = names[..count]
            .iter()
            .map(|name| (name.as_str(), 0, 0))
            .collect();
        layout(&places)
    };
    let far_start = PipelineLayout {
        start: Some(Position { x: 0, y: 1_000_001 }),
        ..PipelineLayout::default()
    };

    // when
    let none = app.save_pipeline_layout(owner, &id, None).await;
    let far_x = layout(&[("build", 1_000_001, 0)]);
    let far_x = app.save_pipeline_layout(owner, &id, Some(far_x)).await;
    let far_y = layout(&[("build", 0, -1_000_001)]);
    let far_y = app.save_pipeline_layout(owner, &id, Some(far_y)).await;
    let far_start = app.save_pipeline_layout(owner, &id, Some(far_start)).await;
    let too_many = app
        .save_pipeline_layout(owner, &id, Some(places(257)))
        .await;
    let most = app
        .save_pipeline_layout(owner, &id, Some(places(256)))
        .await;
    let edge = layout(&[("build", 1_000_000, -1_000_000)]);
    let edge = app.save_pipeline_layout(owner, &id, Some(edge)).await;

    // then
    assert_invalid_field(&none.unwrap_err(), "layout");
    assert_invalid_field(&far_x.unwrap_err(), "layout.nodes[build]");
    assert_invalid_field(&far_y.unwrap_err(), "layout.nodes[build]");
    assert_invalid_field(&far_start.unwrap_err(), "layout.start");
    assert_invalid_field(&too_many.unwrap_err(), "layout.nodes");
    assert!(most.is_ok() && edge.is_ok());
}

/// The members of the workspace and the editors of the project save places. A viewer reads them
/// and cannot save. Another user and an unknown pipeline get `PERMISSION_DENIED`.
#[sqlx::test(migrations = false)]
async fn save_pipeline_layout_follows_the_roles(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project, created) = pipeline(&app).await;
    let id = text(&created.id).to_owned();
    let (member, editor, viewer, stranger) = (
        app.user().await,
        app.user().await,
        app.user().await,
        app.user().await,
    );
    let workspace = app.workspace_of(owner).await;
    let project_id = ProjectId::parse(&project).unwrap();
    app.grant(&[
        Workspace::MEMBER.of(workspace, member),
        Project::EDITOR.of(project_id, editor),
        Project::VIEWER.of(project_id, viewer),
    ])
    .await;
    let place = || Some(layout(&[("build", 1, 2)]));

    // when
    let by_owner = app.save_pipeline_layout(owner, &id, place()).await;
    let by_member = app.save_pipeline_layout(member, &id, place()).await;
    let by_editor = app.save_pipeline_layout(editor, &id, place()).await;
    let by_viewer = app.save_pipeline_layout(viewer, &id, place()).await;
    let by_stranger = app.save_pipeline_layout(stranger, &id, place()).await;
    let unknown = app.save_pipeline_layout(owner, UNKNOWN_ID, place()).await;
    let read = app.pipeline_layout(viewer, &id).await.unwrap();

    // then
    assert!(by_owner.is_ok() && by_member.is_ok() && by_editor.is_ok());
    assert_code(by_viewer, Code::PermissionDenied);
    assert_code(by_stranger, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
    assert_eq!(read.unwrap().nodes, place().unwrap().nodes);
}
