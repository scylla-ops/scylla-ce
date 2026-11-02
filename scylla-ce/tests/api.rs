use std::collections::BTreeSet;

use scylla_proto::projects::v1::CreateProjectRequest;
use scylla_testkit::TestApp;
use sqlx::PgPool;
use tonic::Code;

const UNKNOWN_ID: &str = "00000000-0000-0000-0000-000000000000";

async fn app(pool: PgPool) -> TestApp {
    TestApp::spawn(pool, scylla_ce::edition()).await
}

#[sqlx::test(migrations = false)]
async fn creates_twenty_projects(pool: PgPool) {
    let app = app(pool).await;
    let owner = app.user("owner").await;

    for i in 0..20 {
        app.create_project(owner, &format!("project {i}"))
            .await
            .expect("the community edition has no limit");
    }

    let page = app.list_projects(owner, 100, "").await.unwrap();
    assert_eq!(page.projects.len(), 20);
    assert!(page.next_page_token.is_empty());
    assert_eq!(app.project_count(owner).await, 20);
}

#[sqlx::test(migrations = false)]
async fn lists_projects_in_pages(pool: PgPool) {
    let app = app(pool).await;
    let owner = app.user("owner").await;
    for i in 0..20 {
        app.create_project(owner, &format!("project {i}"))
            .await
            .unwrap();
    }

    let (mut ids, mut sizes, mut token) = (Vec::new(), Vec::new(), String::new());
    loop {
        let page = app.list_projects(owner, 8, &token).await.unwrap();
        sizes.push(page.projects.len());
        ids.extend(page.projects.into_iter().map(|project| project.id));
        if page.next_page_token.is_empty() {
            break;
        }
        token = page.next_page_token;
    }

    assert_eq!(sizes, vec![8, 8, 4]);
    assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), 20);
}

#[sqlx::test(migrations = false)]
async fn rejects_invalid_project_requests(pool: PgPool) {
    let app = app(pool).await;
    let owner = app.user("owner").await;
    let create = |owner_id: &str, name: &str| CreateProjectRequest {
        owner_id: owner_id.to_owned(),
        name: name.to_owned(),
    };
    let mut projects = app.projects.clone();
    app.create_project(owner, &"x".repeat(100))
        .await
        .expect("100 characters is the maximum");

    let empty_name = app.create_project(owner, "   ").await.unwrap_err();
    let long_name = app
        .create_project(owner, &"x".repeat(101))
        .await
        .unwrap_err();
    let bad_owner = projects
        .create_project(create("nope", "x"))
        .await
        .unwrap_err();
    let unknown_owner = projects
        .create_project(create(UNKNOWN_ID, "x"))
        .await
        .unwrap_err();
    let unknown_project = app.delete_project(UNKNOWN_ID).await.unwrap_err();
    let negative_page = app.list_projects(owner, -1, "").await.unwrap_err();

    assert_eq!(empty_name.code(), Code::InvalidArgument);
    assert_eq!(long_name.code(), Code::InvalidArgument);
    assert_eq!(bad_owner.code(), Code::InvalidArgument);
    assert_eq!(unknown_owner.code(), Code::NotFound);
    assert_eq!(unknown_project.code(), Code::NotFound);
    assert_eq!(negative_page.code(), Code::InvalidArgument);
}

#[sqlx::test(migrations = false)]
async fn reports_the_community_edition(pool: PgPool) {
    let app = app(pool).await;

    let capabilities = app.get_capabilities().await.unwrap();

    assert_eq!(capabilities.edition, "community");
    assert!(capabilities.capabilities.is_empty());
}
