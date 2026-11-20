//! The tests of `ListPipelineSummaries`: the order and the pages, the pipelines that each caller
//! sees, and the bad values of a request.

use scylla_proto::pipelines::v1::{ListPipelineSummariesRequest, PipelineSummaryOrder};
use scylla_testkit::{id, text, Role};
use sqlx::PgPool;

use super::pipeline_summaries::{job, named_pipeline, names, request};
use crate::{app, assert_invalid_field};

/// The failing pipelines come first by default, and the pages follow the order of the request.
/// A filter of pipelines keeps only these pipelines.
#[sqlx::test(migrations = false)]
async fn list_pipeline_summaries_sorts_and_pages(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let project = text(&project.id).to_owned();
    let green = named_pipeline(&app, owner, &project, "api").await;
    let red = named_pipeline(&app, owner, &project, "deploy").await;
    named_pipeline(&app, owner, &project, "Build").await;
    job(&app, owner, &green, "completed", 60.0).await;
    job(&app, owner, &red, "failed", 5.0).await;

    // when
    let failing = app
        .list_pipeline_summaries(owner, request(workspace))
        .await
        .unwrap();
    let mut slowest = request(workspace);
    slowest.order = PipelineSummaryOrder::Slowest as i32;
    let slowest = app.list_pipeline_summaries(owner, slowest).await.unwrap();
    let mut first = request(workspace);
    first.order = PipelineSummaryOrder::Name as i32;
    first.page = Some(scylla_proto::common::v1::PageRequest {
        size: 2,
        token: String::new(),
    });
    let page_one = app
        .list_pipeline_summaries(owner, first.clone())
        .await
        .unwrap();
    let mut second = first;
    second.page = page_one
        .page
        .clone()
        .map(|page| scylla_proto::common::v1::PageRequest {
            size: 2,
            token: page.next_token,
        });
    let page_two = app.list_pipeline_summaries(owner, second).await.unwrap();
    let mut only = request(workspace);
    only.pipeline_ids = vec![id(red.clone())];
    let only = app.list_pipeline_summaries(owner, only).await.unwrap();

    // then
    assert_eq!(names(&failing.pipelines), ["deploy", "Build", "api"]);
    assert_eq!((failing.total, failing.green), (3, 1));
    assert_eq!(names(&slowest.pipelines), ["api", "Build", "deploy"]);
    assert_eq!(names(&page_one.pipelines), ["api", "Build"]);
    assert_eq!(names(&page_two.pipelines), ["deploy"]);
    assert_eq!(page_two.page.unwrap().next_token, "");
    assert_eq!(names(&only.pipelines), ["deploy"]);
    assert_eq!(only.total, 1);
}

/// A viewer of one project gets the pipelines of that project only, a reader of the workspace
/// gets each pipeline, and an actor with no role gets an empty list. A filter on a project that
/// the caller cannot see causes no refusal.
#[sqlx::test(migrations = false)]
async fn list_pipeline_summaries_shows_the_visible_projects(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let (viewer, reader, stranger) = (app.user().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    let alpha = app.create_project(owner, "alpha").await.unwrap();
    let beta = app.create_project(owner, "beta").await.unwrap();
    named_pipeline(&app, owner, text(&alpha.id), "web").await;
    named_pipeline(&app, owner, text(&beta.id), "api").await;
    app.join_workspace(owner, viewer).await;
    let role = Role::on("project", text(&alpha.id), "viewer");
    app.grant_role(owner, role, &viewer.to_string())
        .await
        .unwrap();
    let workspace_text = workspace.to_string();
    let role = Role::on("workspace", &workspace_text, "viewer");
    app.grant_role(owner, role, &reader.to_string())
        .await
        .unwrap();

    // when
    let of_owner = app.list_pipeline_summaries(owner, request(workspace)).await;
    let of_viewer = app
        .list_pipeline_summaries(viewer, request(workspace))
        .await;
    let of_reader = app
        .list_pipeline_summaries(reader, request(workspace))
        .await;
    let of_stranger = app
        .list_pipeline_summaries(stranger, request(workspace))
        .await;
    let mut hidden = request(workspace);
    hidden.project_ids = vec![beta.id.clone().unwrap()];
    let hidden = app.list_pipeline_summaries(viewer, hidden).await;

    // then
    assert_eq!(of_owner.unwrap().total, 2);
    let of_viewer = of_viewer.unwrap();
    assert_eq!(names(&of_viewer.pipelines), ["web"]);
    assert_eq!(of_viewer.total, 1);
    assert_eq!(of_reader.unwrap().total, 2);
    assert!(of_stranger.unwrap().pipelines.is_empty());
    assert!(hidden.unwrap().pipelines.is_empty());
}

/// A run count above 50, more than 100 ids in a filter, an unknown order and a request with no
/// workspace give `INVALID_ARGUMENT` on their field.
#[sqlx::test(migrations = false)]
async fn list_pipeline_summaries_refuses_bad_values(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let mut runs = request(workspace);
    runs.runs = 51;
    let mut projects = request(workspace);
    projects.project_ids = (0..101).map(|_| id(workspace.to_string())).collect();
    let mut order = request(workspace);
    order.order = 99;
    let no_workspace = ListPipelineSummariesRequest::default();

    // when
    let runs = app.list_pipeline_summaries(owner, runs).await;
    let projects = app.list_pipeline_summaries(owner, projects).await;
    let order = app.list_pipeline_summaries(owner, order).await;
    let no_workspace = app.list_pipeline_summaries(owner, no_workspace).await;

    // then
    assert_invalid_field(&runs.unwrap_err(), "runs");
    assert_invalid_field(&projects.unwrap_err(), "project_ids");
    assert_invalid_field(&order.unwrap_err(), "order");
    assert_invalid_field(&no_workspace.unwrap_err(), "workspace_id");
}
