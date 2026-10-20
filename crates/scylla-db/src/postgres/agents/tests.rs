use super::PgAgentRepository;
use crate::domain::agent::{Agent, AgentHost};
use crate::domain::app::{App, AppCredential};
use crate::domain::app::{AppName, AppSecretHash, AppSecretLabel};
use crate::domain::clock;
use crate::domain::ids::{AppId, JobId, OrganizationId};
use crate::domain::job::Job;
use crate::domain::job::JobStatus;
use crate::domain::role::RoleName;
use crate::postgres::{PgAppRepository, PgGrantRepository, PgJobRepository};
use crate::test_support::prelude::*;
use scylla_auth::authz::{Grant, GrantRepository, ORGANIZATION_AGENT_ROLE, Principal, Scope};
use scylla_core::application::{AgentRepository, AppRepository, JobRepository};
use sqlx::PgPool;

const TEST_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHQ$aGFzaGhhc2g";

fn default_credential(app: &App) -> AppCredential {
    AppCredential::create(
        app.id().clone(),
        AppSecretLabel::new("default").unwrap(),
        AppSecretHash::new(TEST_HASH).unwrap(),
    )
}

fn make_agent(org_id: &OrganizationId, name: &str) -> (App, AppCredential, Agent, Grant) {
    let app = App::create(org_id.clone(), AppName::new(name).unwrap()).unwrap();
    let credential = default_credential(&app);
    let agent = Agent::create(app.id().clone());
    let grant = Grant::new(
        Principal::App(app.id().clone()),
        RoleName::new(ORGANIZATION_AGENT_ROLE).unwrap(),
        Scope::Organization(org_id.clone()),
    );
    (app, credential, agent, grant)
}

#[sqlx::test(migrations = "../../migrations")]
async fn provision_agent_persists_app_row_and_grant(pool: PgPool) {
    let org = seed_org(&pool, "Acme").await;
    let app_repo = PgAppRepository::new(pool.clone());
    let agent_repo = PgAgentRepository::new(pool.clone());
    let (app, credential, agent, grant) = make_agent(org.id(), "ci-runner");

    app_repo
        .provision_agent(&app, &credential, &agent, &grant)
        .await
        .expect("provision_agent");

    let found = agent_repo
        .find_by_app_id(app.id())
        .await
        .expect("agent row");
    assert_eq!(found.app_id(), app.id());
    assert!(found.last_seen().is_none(), "fresh agent never seen");

    let list = agent_repo.list_by_organization(org.id()).await.unwrap();
    assert_eq!(list.len(), 1);

    let grants = PgGrantRepository::new(pool.clone())
        .list_all()
        .await
        .unwrap();
    assert!(
        grants
            .iter()
            .any(|g| g.principal == Principal::App(app.id().clone())),
        "agent grant minted with the app"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn plain_app_is_not_a_agent(pool: PgPool) {
    let org = seed_org(&pool, "Acme").await;
    let app_repo = PgAppRepository::new(pool.clone());
    let agent_repo = PgAgentRepository::new(pool.clone());

    let app = App::create(org.id().clone(), AppName::new("bot").unwrap()).unwrap();
    let credential = default_credential(&app);
    app_repo
        .create_app(&app, &credential)
        .await
        .expect("create_app");

    assert!(
        agent_repo.find_by_app_id(app.id()).await.is_err(),
        "a plain app has no agents row"
    );
    assert!(
        agent_repo
            .list_by_organization(org.id())
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn touch_last_seen_upserts_and_self_heals(pool: PgPool) {
    let org = seed_org(&pool, "Acme").await;
    let app_repo = PgAppRepository::new(pool.clone());
    let agent_repo = PgAgentRepository::new(pool.clone());

    let app = App::create(org.id().clone(), AppName::new("legacy").unwrap()).unwrap();
    let credential = default_credential(&app);
    app_repo.create_app(&app, &credential).await.unwrap();

    let t1 = clock::now();
    agent_repo
        .touch_last_seen(app.id(), t1)
        .await
        .expect("self-heal insert");
    let after_first = agent_repo.find_by_app_id(app.id()).await.unwrap();
    assert!(after_first.last_seen().is_some());

    let t2 = t1 + chrono::Duration::seconds(30);
    agent_repo
        .touch_last_seen(app.id(), t2)
        .await
        .expect("upsert update");
    let after_second = agent_repo.find_by_app_id(app.id()).await.unwrap();
    assert_eq!(
        after_second.last_seen().unwrap().timestamp(),
        t2.timestamp()
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn touch_last_seen_of_a_deleted_app_is_a_no_op(pool: PgPool) {
    let agent_repo = PgAgentRepository::new(pool.clone());
    let gone = AppId::new("gone");

    agent_repo
        .touch_last_seen(&gone, clock::now())
        .await
        .unwrap();

    assert!(agent_repo.find_by_app_id(&gone).await.is_err());
}

#[sqlx::test(migrations = "../../migrations")]
async fn record_host_upserts_and_fully_overwrites(pool: PgPool) {
    let org = seed_org(&pool, "Acme").await;
    let app_repo = PgAppRepository::new(pool.clone());
    let agent_repo = PgAgentRepository::new(pool.clone());

    let app = App::create(org.id().clone(), AppName::new("legacy").unwrap()).unwrap();
    app_repo
        .create_app(&app, &default_credential(&app))
        .await
        .unwrap();
    assert!(agent_repo.find_by_app_id(app.id()).await.is_err());

    let first = AgentHost {
        version: "0.4.0".into(),
        os: "linux".into(),
        arch: "x86_64".into(),
        hostname: "builder-01".into(),
        cpu_count: Some(8),
        total_memory_mb: Some(16_384),
        reported_at: clock::now(),
    };
    agent_repo.record_host(app.id(), &first).await.unwrap();
    assert_eq!(
        agent_repo.find_by_app_id(app.id()).await.unwrap().host(),
        Some(&first)
    );

    let moved = AgentHost {
        version: "0.5.0".into(),
        os: "linux".into(),
        arch: "aarch64".into(),
        hostname: "builder-02".into(),
        cpu_count: None,
        total_memory_mb: None,
        reported_at: clock::now(),
    };
    agent_repo.record_host(app.id(), &moved).await.unwrap();
    assert_eq!(
        agent_repo.find_by_app_id(app.id()).await.unwrap().host(),
        Some(&moved)
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn agent_that_never_said_hello_has_no_host(pool: PgPool) {
    let org = seed_org(&pool, "Acme").await;
    let app_repo = PgAppRepository::new(pool.clone());
    let agent_repo = PgAgentRepository::new(pool.clone());
    let (app, credential, agent, grant) = make_agent(org.id(), "silent");
    app_repo
        .provision_agent(&app, &credential, &agent, &grant)
        .await
        .unwrap();

    agent_repo
        .touch_last_seen(app.id(), clock::now())
        .await
        .unwrap();

    let found = agent_repo.find_by_app_id(app.id()).await.unwrap();
    assert!(found.last_seen().is_some(), "presence still tracked");
    assert!(found.host().is_none(), "no host without a hello");
}

#[sqlx::test(migrations = "../../migrations")]
async fn agent_stats_aggregate_jobs_by_status(pool: PgPool) {
    let (org, _project, pipeline) = seed_org_project_pipeline(&pool, "stats").await;
    let app_repo = PgAppRepository::new(pool.clone());
    let agent_repo = PgAgentRepository::new(pool.clone());
    let job_repo = PgJobRepository::new(pool.clone());
    let (app, credential, agent, grant) = make_agent(org.id(), "runner");
    app_repo
        .provision_agent(&app, &credential, &agent, &grant)
        .await
        .unwrap();

    let mk = |status: JobStatus| {
        let now = clock::now();
        let (started_at, finished_at) = match status {
            JobStatus::Pending => (None, None),
            JobStatus::Running => (Some(now), None),
            JobStatus::Completed
            | JobStatus::Failed
            | JobStatus::Cancelled
            | JobStatus::Orphaned => (Some(now - chrono::Duration::seconds(1)), Some(now)),
        };
        let state =
            crate::domain::job::JobState::from_columns(status, started_at, finished_at).unwrap();
        Job::from_persistence(
            JobId::generate(),
            pipeline.id().clone(),
            state,
            Some(app.id().clone()),
            vec![],
            vec![],
            vec![],
            crate::domain::job::JobOrigin::App {
                app_id: app.id().clone(),
            },
            now,
            now,
            0,
        )
    };
    for status in [
        JobStatus::Completed,
        JobStatus::Completed,
        JobStatus::Failed,
        JobStatus::Running,
    ] {
        job_repo.create(&mk(status)).await.unwrap();
    }

    let stats = agent_repo.agent_stats(app.id()).await.unwrap();
    assert_eq!(stats.total, 4);
    assert_eq!(stats.completed, 2);
    assert_eq!(stats.failed, 1);
    assert_eq!(stats.running, 1);
    assert!(stats.last_run_at.is_some());
}

#[sqlx::test(migrations = "../../migrations")]
async fn agent_stats_partition_total_and_summarize_durations(pool: PgPool) {
    let (org, _project, pipeline) = seed_org_project_pipeline(&pool, "durations").await;
    let app_repo = PgAppRepository::new(pool.clone());
    let agent_repo = PgAgentRepository::new(pool.clone());
    let job_repo = PgJobRepository::new(pool.clone());
    let (app, credential, agent, grant) = make_agent(org.id(), "runner");
    app_repo
        .provision_agent(&app, &credential, &agent, &grant)
        .await
        .unwrap();

    let now = clock::now();
    let mk = |status: JobStatus, ran_for: Option<i64>| {
        let (started_at, finished_at) = match (status, ran_for) {
            (JobStatus::Pending, _) => (None, None),
            (JobStatus::Running, _) => (Some(now), None),
            (_, Some(ms)) => (Some(now - chrono::Duration::milliseconds(ms)), Some(now)),
            (_, None) => (None, Some(now)),
        };
        let state =
            crate::domain::job::JobState::from_columns(status, started_at, finished_at).unwrap();
        Job::from_persistence(
            JobId::generate(),
            pipeline.id().clone(),
            state,
            Some(app.id().clone()),
            vec![],
            vec![],
            vec![],
            crate::domain::job::JobOrigin::App {
                app_id: app.id().clone(),
            },
            now,
            now,
            0,
        )
    };

    for (status, ran_for) in [
        (JobStatus::Completed, Some(1000)),
        (JobStatus::Completed, Some(3000)),
        (JobStatus::Failed, Some(2000)),
        (JobStatus::Orphaned, Some(4000)),
        (JobStatus::Cancelled, None),
        (JobStatus::Pending, None),
    ] {
        job_repo.create(&mk(status, ran_for)).await.unwrap();
    }

    let stats = agent_repo.agent_stats(app.id()).await.unwrap();

    assert_eq!(stats.total, 6);
    assert_eq!(
        stats.pending
            + stats.running
            + stats.completed
            + stats.failed
            + stats.cancelled
            + stats.orphaned,
        stats.total,
        "status buckets must sum back to total"
    );
    assert_eq!(stats.orphaned, 1);
    assert_eq!(stats.cancelled, 1);

    assert_eq!(stats.median_duration_ms, Some(2500));
    assert_eq!(stats.p95_duration_ms, Some(3850));

    assert_eq!(stats.daily.len(), 1);
    assert_eq!(stats.daily[0].median_duration_ms, Some(2500));
    assert_eq!(stats.daily[0].orphaned, 1, "daily carries orphaned too");
}

#[sqlx::test(migrations = "../../migrations")]
async fn agent_stats_report_no_duration_when_nothing_ran(pool: PgPool) {
    let (org, _project, pipeline) = seed_org_project_pipeline(&pool, "nodur").await;
    let app_repo = PgAppRepository::new(pool.clone());
    let agent_repo = PgAgentRepository::new(pool.clone());
    let job_repo = PgJobRepository::new(pool.clone());
    let (app, credential, agent, grant) = make_agent(org.id(), "runner");
    app_repo
        .provision_agent(&app, &credential, &agent, &grant)
        .await
        .unwrap();

    let job = JobBuilder::new(&pipeline).agent(app.id().clone()).build();
    job_repo.create(&job).await.unwrap();

    let stats = agent_repo.agent_stats(app.id()).await.unwrap();
    assert_eq!(stats.total, 1);
    assert_eq!(stats.pending, 1);
    assert_eq!(stats.median_duration_ms, None);
    assert_eq!(stats.p95_duration_ms, None);
}

#[sqlx::test(migrations = "../../migrations")]
async fn deleting_agent_keeps_jobs_and_nulls_attribution(pool: PgPool) {
    let (org, _project, pipeline) = seed_org_project_pipeline(&pool, "del").await;
    let app_repo = PgAppRepository::new(pool.clone());
    let agent_repo = PgAgentRepository::new(pool.clone());
    let job_repo = PgJobRepository::new(pool.clone());
    let (app, credential, agent, grant) = make_agent(org.id(), "runner");
    app_repo
        .provision_agent(&app, &credential, &agent, &grant)
        .await
        .unwrap();

    let job = JobBuilder::new(&pipeline).agent(app.id().clone()).build();
    let job = job_repo.create(&job).await.unwrap();

    app_repo.delete(app.id()).await.unwrap();

    assert!(agent_repo.find_by_app_id(app.id()).await.is_err());
    let reloaded = job_repo.find_by_id(job.id()).await.expect("job survives");
    assert!(
        reloaded.agent_app_id().is_none(),
        "attribution nulled on agent delete"
    );
}
