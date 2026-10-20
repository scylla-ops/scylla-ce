use super::PgJobLogRepository;
use crate::domain::job::JobLog;
use crate::postgres::PgJobRepository;
use crate::test_support::prelude::*;
use chrono::{Duration, Utc};
use scylla_core::application::{JobLogRepository, JobRepository};
use sqlx::PgPool;

#[sqlx::test(migrations = "../../migrations")]
async fn create_then_list_by_job(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "l").await;
    let job = seed_job(&pool, &pipeline).await;
    let repo = PgJobLogRepository::new(pool);

    let log = job_log(job.id(), "a", "hello");
    repo.create_many(std::slice::from_ref(&log)).await.unwrap();

    let listed = repo.list_all_by_job(job.id(), None).await.unwrap();
    let [found] = listed.as_slice() else {
        panic!("expected one log, got {}", listed.len());
    };
    assert_eq!(found.id(), log.id());
    assert_eq!(found.line(), "hello");
    assert_eq!(found.node_id().as_str(), "a");
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_by_job_orders_by_timestamp_ascending(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "ord").await;
    let job = seed_job(&pool, &pipeline).await;
    let repo = PgJobLogRepository::new(pool);

    let base = Utc::now();
    let logs: Vec<JobLog> = [("third", 0_i64), ("second", -1), ("first", -2)]
        .into_iter()
        .map(|(line, offset_secs)| {
            JobLogBuilder::new(job.id(), "a", line)
                .timestamp(base + Duration::seconds(offset_secs))
                .build()
        })
        .collect();
    repo.create_many(&logs).await.unwrap();

    let listed = repo.list_all_by_job(job.id(), None).await.unwrap();
    let lines: Vec<&str> = listed.iter().map(JobLog::line).collect();
    assert_eq!(lines, vec!["first", "second", "third"]);
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_by_job_and_node_filters_other_nodes(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "f").await;
    let job = seed_job(&pool, &pipeline).await;
    let repo = PgJobLogRepository::new(pool);

    repo.create_many(&[
        job_log(job.id(), "a", "log-a-1"),
        job_log(job.id(), "a", "log-a-2"),
        job_log(job.id(), "b", "log-b-1"),
    ])
    .await
    .unwrap();

    let target = crate::domain::pipeline::NodeId::new("a").unwrap();
    let scoped = repo.list_all_by_job(job.id(), Some(&target)).await.unwrap();
    assert_eq!(scoped.len(), 2);
    assert!(scoped.iter().all(|l| l.node_id().as_str() == "a"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_all_by_job_returns_full_history(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "h").await;
    let job = seed_job(&pool, &pipeline).await;
    let repo = PgJobLogRepository::new(pool);

    for i in 0..5 {
        repo.create_many(&[job_log(job.id(), "a", &format!("line-{i}"))])
            .await
            .unwrap();
    }

    assert_eq!(repo.list_all_by_job(job.id(), None).await.unwrap().len(), 5);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_batch_is_stored_whole_in_its_order(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "b").await;
    let job = seed_job(&pool, &pipeline).await;
    let repo = PgJobLogRepository::new(pool);
    let base = Utc::now();
    let logs: Vec<JobLog> = (0..1000)
        .map(|i| {
            JobLogBuilder::new(job.id(), "a", format!("line-{i}"))
                .timestamp(base + Duration::microseconds(i))
                .build()
        })
        .collect();

    repo.create_many(&logs).await.unwrap();
    repo.create_many(&[]).await.unwrap();

    let listed = repo.list_all_by_job(job.id(), None).await.unwrap();
    assert_eq!(listed.len(), 1000);
    assert_eq!(listed[0].line(), "line-0");
    assert_eq!(listed[999].line(), "line-999");
}

#[sqlx::test(migrations = "../../migrations")]
async fn cascade_job_delete_removes_logs(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "c").await;
    let job = seed_job(&pool, &pipeline).await;
    let log_repo = PgJobLogRepository::new(pool.clone());
    log_repo
        .create_many(&[job_log(job.id(), "a", "doomed")])
        .await
        .unwrap();

    PgJobRepository::new(pool).delete(&job).await.unwrap();

    assert!(
        log_repo
            .list_all_by_job(job.id(), None)
            .await
            .unwrap()
            .is_empty()
    );
}
