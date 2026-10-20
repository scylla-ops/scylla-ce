use super::PgPipelineRepository;
use crate::domain::errors::DomainError;
use crate::domain::pipeline::NodeId;
use crate::postgres::PgProjectRepository;
use crate::test_support::prelude::*;
use scylla_core::application::{PipelineRepository, ProjectRepository};
use sqlx::PgPool;

#[sqlx::test(migrations = "../../migrations")]
async fn nodes_jsonb_round_trips_exactly(pool: PgPool) {
    let org = seed_org(&pool, "acme").await;
    let project = seed_project(&pool, &org, "rocket").await;
    let repo = PgPipelineRepository::new(pool);

    let pipeline = PipelineBuilder::new(&project)
        .nodes(vec![
            node("a", &[]),
            node("b", &["a"]),
            node("c", &["a", "b"]),
        ])
        .build();
    repo.create(&pipeline).await.unwrap();

    let found = repo.find_by_id(pipeline.id()).await.unwrap();
    assert_eq!(found.nodes().len(), 3);
    let deps_of_c: Vec<&str> = found
        .nodes()
        .iter()
        .find(|n| n.id().as_str() == "c")
        .unwrap()
        .deps()
        .iter()
        .map(NodeId::as_str)
        .collect();
    assert_eq!(deps_of_c, vec!["a", "b"]);
}

#[sqlx::test(migrations = "../../migrations")]
async fn update_nodes_persists_new_dag(pool: PgPool) {
    let (_, _, mut pipeline) = seed_org_project_pipeline(&pool, "u").await;
    let repo = PgPipelineRepository::new(pool);

    pipeline
        .update_nodes(vec![node("x", &[]), node("y", &["x"])])
        .unwrap();
    repo.update(&pipeline).await.unwrap();

    let found = repo.find_by_id(pipeline.id()).await.unwrap();
    let ids: Vec<&str> = found.nodes().iter().map(|n| n.id().as_str()).collect();
    assert_eq!(ids, vec!["x", "y"]);
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_by_project_filters(pool: PgPool) {
    let org = seed_org(&pool, "acme").await;
    let project_a = seed_project(&pool, &org, "a").await;
    let project_b = seed_project(&pool, &org, "b").await;
    let repo = PgPipelineRepository::new(pool);

    repo.create(&pipeline(&project_a)).await.unwrap();
    repo.create(&pipeline(&project_b)).await.unwrap();

    assert_eq!(
        repo.list_by_project(project_a.id(), None)
            .await
            .unwrap()
            .metadata()
            .total_count(),
        1,
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_by_organization_joins_through_projects(pool: PgPool) {
    let org_target = seed_org(&pool, "target").await;
    let org_other = seed_org(&pool, "other").await;
    let p1 = seed_project(&pool, &org_target, "p1").await;
    let p2 = seed_project(&pool, &org_target, "p2").await;
    let p3 = seed_project(&pool, &org_other, "p3").await;
    let repo = PgPipelineRepository::new(pool);

    for project in [&p1, &p2, &p3] {
        repo.create(&pipeline(project)).await.unwrap();
    }

    assert_eq!(
        repo.list_by_organization(org_target.id(), None)
            .await
            .unwrap()
            .metadata()
            .total_count(),
        2,
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn cascade_project_delete_removes_pipelines(pool: PgPool) {
    let (_, project, pipeline) = seed_org_project_pipeline(&pool, "c").await;
    let repo = PgPipelineRepository::new(pool.clone());

    PgProjectRepository::new(pool)
        .delete(&project)
        .await
        .unwrap();

    assert!(matches!(
        repo.find_by_id(pipeline.id()).await,
        Err(DomainError::NotFound(_)),
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn delete_then_find_returns_not_found(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "d").await;
    let repo = PgPipelineRepository::new(pool);

    repo.delete(&pipeline).await.unwrap();
    assert!(matches!(
        repo.find_by_id(pipeline.id()).await,
        Err(DomainError::NotFound(_)),
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_update_from_a_stale_read_is_stale_and_the_first_write_wins(pool: PgPool) {
    use crate::domain::pipeline::PipelineName;

    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "v").await;
    let repo = PgPipelineRepository::new(pool);

    let mut first = repo.find_by_id(pipeline.id()).await.expect("first read");
    let mut second = repo.find_by_id(pipeline.id()).await.expect("second read");
    assert_eq!(first.version(), 0);

    first
        .update_name(PipelineName::new("renamed").unwrap())
        .unwrap();
    let written = repo.update(&first).await.expect("first write");
    assert_eq!(written.version(), 1);

    second.update_nodes(vec![node("other", &[])]).unwrap();
    let err = repo.update(&second).await.expect_err("stale write");
    assert!(matches!(err, DomainError::Stale(_)));

    let stored = repo.find_by_id(pipeline.id()).await.expect("find");
    assert_eq!(stored.name().as_str(), "renamed");
    assert_eq!(
        stored.nodes()[0].id().as_str(),
        pipeline.nodes()[0].id().as_str()
    );
    assert_eq!(stored.version(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_delete_with_a_stale_version_is_stale(pool: PgPool) {
    use crate::domain::pipeline::PipelineName;

    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "v").await;
    let repo = PgPipelineRepository::new(pool);

    let stale = repo.find_by_id(pipeline.id()).await.expect("read");
    let mut fresh = stale.clone();
    fresh
        .update_name(PipelineName::new("renamed").unwrap())
        .unwrap();
    let fresh = repo.update(&fresh).await.expect("write");

    let err = repo.delete(&stale).await.expect_err("stale delete");
    assert!(matches!(err, DomainError::Stale(_)));
    assert!(repo.find_by_id(pipeline.id()).await.is_ok());

    repo.delete(&fresh)
        .await
        .expect("delete at the current version");
    assert!(matches!(
        repo.find_by_id(pipeline.id()).await,
        Err(DomainError::NotFound(_))
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_write_on_a_missing_row_is_not_found(pool: PgPool) {
    let org = seed_org(&pool, "acme").await;
    let project = seed_project(&pool, &org, "rocket").await;
    let repo = PgPipelineRepository::new(pool);
    let never_persisted = pipeline(&project);

    assert!(matches!(
        repo.update(&never_persisted).await,
        Err(DomainError::NotFound(_))
    ));
    assert!(matches!(
        repo.delete(&never_persisted).await,
        Err(DomainError::NotFound(_))
    ));
}
