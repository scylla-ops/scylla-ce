//! The tests of the search, the health filter, the order, the ids and the counts of
//! `ListProjects`.

use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::common::v1::PageRequest;
use scylla_proto::projects::v1::{
    ListProjectsRequest, ListProjectsResponse, ProjectCounts, ProjectHealth, ProjectOrder,
};
use scylla_testkit::{id, text, LinkRow};
use sqlx::PgPool;
use tonic::Code;

use super::pipeline_summaries::{job, named_pipeline};
use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// Makes a request for the projects of a workspace, with a search text, an order and a health.
fn request(
    workspace: WorkspaceId,
    query: &str,
    order: ProjectOrder,
    health: ProjectHealth,
) -> ListProjectsRequest {
    ListProjectsRequest {
        workspace_id: Some(id(workspace.to_string())),
        query: query.to_owned(),
        order: order.into(),
        health: health.into(),
        ..ListProjectsRequest::default()
    }
}

/// Returns the names of the projects of a list, in its order.
fn names(list: &ListProjectsResponse) -> Vec<&str> {
    list.projects
        .iter()
        .map(|project| project.name.as_str())
        .collect()
}

/// The search reads the name, the description, the linked repository and the names of the
/// pipelines. The health keeps the failing or the running projects, and the counts of the search
/// do not change with the health.
#[sqlx::test(migrations = false)]
async fn list_projects_searches_and_filters_by_health(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let web = app.create_project(owner, "Web").await.unwrap();
    let red = named_pipeline(&app, owner, text(&web.id), "CI").await;
    job(&app, owner, &red, "failed", 3.0).await;
    named_pipeline(&app, owner, text(&web.id), "E2E").await;
    let cloud = app.create_project(owner, "cloud").await.unwrap();
    let connection = app
        .insert_github_connection(workspace, owner, 7, "octo")
        .await;
    app.insert_repository_link(&LinkRow {
        project: text(&cloud.id),
        connection: &connection,
        external_id: "42",
        full_name: "octo/cloud-web",
        default_branch: "main",
    })
    .await;
    let api = app.create_project(owner, "api").await.unwrap();
    let busy = named_pipeline(&app, owner, text(&api.id), "Web deploy").await;
    job(&app, owner, &busy, "pending", 0.0).await;
    app.create_project(owner, "cli").await.unwrap();

    // when
    let all = ProjectHealth::All;
    let found = app
        .search_projects(owner, request(workspace, " WEB ", ProjectOrder::Name, all))
        .await
        .unwrap();
    let failed = request(workspace, "web", ProjectOrder::Name, ProjectHealth::Failed);
    let failed = app.search_projects(owner, failed).await.unwrap();
    let running = request(workspace, "", ProjectOrder::Name, ProjectHealth::Running);
    let running = app.search_projects(owner, running).await.unwrap();
    let none = request(workspace, "terraform", ProjectOrder::Name, all);
    let none = app.search_projects(owner, none).await.unwrap();

    // then
    assert_eq!(names(&found), ["api", "cloud", "Web"]);
    let counts = ProjectCounts {
        total: 3,
        failed: 1,
        running: 1,
        pipelines: 3,
    };
    assert_eq!(found.counts, Some(counts));
    assert_eq!(names(&failed), ["Web"]);
    assert_eq!(failed.counts, Some(counts));
    assert_eq!(names(&running), ["api"]);
    assert_eq!(running.counts.unwrap().total, 4);
    assert!(none.projects.is_empty());
    assert_eq!(none.counts, Some(ProjectCounts::default()));
}

/// The orders: oldest first by default, the name, the newest job first, and the failing projects
/// first. The pages follow the order.
#[sqlx::test(migrations = false)]
async fn list_projects_sorts_and_pages(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let mut ids = Vec::new();
    for name in ["delta", "alpha", "charlie", "bravo"] {
        let project = app.create_project(owner, name).await.unwrap();
        ids.push(text(&project.id).to_owned());
    }
    let old = named_pipeline(&app, owner, &ids[2], "build").await;
    job(&app, owner, &old, "completed", 1.0).await;
    let new = named_pipeline(&app, owner, &ids[3], "build").await;
    job(&app, owner, &new, "failed", 1.0).await;
    let all = ProjectHealth::All;

    // when
    let position = request(workspace, "", ProjectOrder::Unspecified, all);
    let position = app.search_projects(owner, position).await.unwrap();
    let name = request(workspace, "", ProjectOrder::Name, all);
    let name = app.search_projects(owner, name).await.unwrap();
    let last_job = request(workspace, "", ProjectOrder::LastJob, all);
    let last_job = app.search_projects(owner, last_job).await.unwrap();
    let failed_first = request(workspace, "", ProjectOrder::FailedFirst, all);
    let failed_first = app.search_projects(owner, failed_first).await.unwrap();
    let mut first = request(workspace, "", ProjectOrder::Name, all);
    first.page = Some(PageRequest {
        size: 3,
        token: String::new(),
    });
    let first = app.search_projects(owner, first.clone()).await.unwrap();
    let mut second = request(workspace, "", ProjectOrder::Name, all);
    second.page = Some(PageRequest {
        size: 3,
        token: first.page.clone().unwrap().next_token,
    });
    let second = app.search_projects(owner, second).await.unwrap();

    // then
    assert_eq!(names(&position), ["delta", "alpha", "charlie", "bravo"]);
    assert_eq!(names(&name), ["alpha", "bravo", "charlie", "delta"]);
    assert_eq!(names(&last_job), ["bravo", "charlie", "alpha", "delta"]);
    assert_eq!(names(&failed_first), ["bravo", "alpha", "charlie", "delta"]);
    assert_eq!(names(&first), ["alpha", "bravo", "charlie"]);
    assert_eq!(names(&second), ["delta"]);
    assert_eq!(second.page.unwrap().next_token, "");
    assert_eq!(first.counts.unwrap().total, 4);
}

/// The ids keep their projects, and the counts read only them.
#[sqlx::test(migrations = false)]
async fn list_projects_keeps_the_projects_of_the_ids(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let alpha = app.create_project(owner, "alpha").await.unwrap();
    app.create_project(owner, "bravo").await.unwrap();
    let charlie = app.create_project(owner, "charlie").await.unwrap();
    let mut pinned = request(workspace, "", ProjectOrder::Name, ProjectHealth::All);
    pinned.project_ids = vec![charlie.id.clone().unwrap(), alpha.id.clone().unwrap()];

    // when
    let list = app.search_projects(owner, pinned).await.unwrap();

    // then
    assert_eq!(names(&list), ["alpha", "charlie"]);
    assert_eq!(list.counts.unwrap().total, 2);
}

/// A search text of more than 100 characters, an unknown order, an unknown health and more than
/// 100 project ids give `INVALID_ARGUMENT`.
#[sqlx::test(migrations = false)]
async fn list_projects_refuses_bad_values(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let all = ProjectHealth::All;
    let long = request(workspace, &"x".repeat(101), ProjectOrder::Name, all);
    let mut order = request(workspace, "", ProjectOrder::Name, all);
    order.order = 99;
    let mut health = request(workspace, "", ProjectOrder::Name, all);
    health.health = 99;
    let mut many = request(workspace, "", ProjectOrder::Name, all);
    many.project_ids = (0..101).map(|_| id(UNKNOWN_ID)).collect();

    // when
    let long = app.search_projects(owner, long).await;
    let order = app.search_projects(owner, order).await;
    let health = app.search_projects(owner, health).await;
    let many = app.search_projects(owner, many).await;

    // then
    assert_code(long, Code::InvalidArgument);
    assert_code(order, Code::InvalidArgument);
    assert_code(health, Code::InvalidArgument);
    assert_invalid_field(&many.unwrap_err(), "project_ids");
}
