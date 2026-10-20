use super::PgJobRepository;
use crate::domain::agent::Agent;
use crate::domain::app::{App, AppCredential, AppName, AppSecretHash, AppSecretLabel};
use crate::domain::clock;
use crate::domain::errors::DomainError;
use crate::domain::ids::{AppId, JobId, OrganizationId, ProjectId, StreamId, TriggerId};
use crate::domain::job::{Job, JobOrigin, JobStatus};
use crate::domain::organization::Organization;
use crate::domain::role::RoleName;
use crate::postgres::{PgAgentRepository, PgAppRepository, PgPipelineRepository};
use crate::test_support::prelude::*;
use chrono::{Duration, Utc};
use scylla_auth::authz::{Grant, ORGANIZATION_AGENT_ROLE, Principal, Scope, Visibility};
use scylla_core::application::agent::AgentStream;
use scylla_core::application::job::JobScope;
use scylla_core::application::pagination::PaginationParams;
use scylla_core::application::{AgentRepository, AppRepository, JobRepository, PipelineRepository};
use sqlx::PgPool;

async fn seed_agent(pool: &PgPool, org: &Organization, name: &str) -> AppId {
    let app = App::create(org.id().clone(), AppName::new(name).unwrap()).unwrap();
    let credential = AppCredential::create(
        app.id().clone(),
        AppSecretLabel::new("default").unwrap(),
        AppSecretHash::new("$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHQ$aGFzaGhhc2g").unwrap(),
    );
    let grant = Grant::new(
        Principal::App(app.id().clone()),
        RoleName::new(ORGANIZATION_AGENT_ROLE).unwrap(),
        Scope::Organization(org.id().clone()),
    );
    PgAppRepository::new(pool.clone())
        .provision_agent(&app, &credential, &Agent::create(app.id().clone()), &grant)
        .await
        .expect("provision agent");
    app.id().clone()
}

fn stream_of(agent: &AppId) -> AgentStream {
    AgentStream {
        agent: agent.clone(),
        id: StreamId::generate(),
    }
}

async fn seen(pool: &PgPool, agent: &AppId, ago: Duration) {
    PgAgentRepository::new(pool.clone())
        .touch_last_seen(agent, clock::now() - ago)
        .await
        .unwrap();
}

async fn status(repo: &PgJobRepository, id: &JobId) -> JobStatus {
    repo.find_by_id(id).await.unwrap().status()
}

fn ids(jobs: &[Job]) -> Vec<JobId> {
    let mut ids: Vec<JobId> = jobs.iter().map(|j| j.id().clone()).collect();
    ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    ids
}

fn sorted(mut ids: Vec<JobId>) -> Vec<JobId> {
    ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    ids
}

#[sqlx::test(migrations = "../../migrations")]
async fn create_then_find_round_trips_the_nodes_and_the_version(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "rt").await;
    let repo = PgJobRepository::new(pool);
    let job = job(&pipeline);
    repo.create(&job).await.unwrap();

    let found = repo.find_by_id(job.id()).await.unwrap();
    assert_eq!(found.node_executions().len(), pipeline.nodes().len());
    let nodes: Vec<&str> = found.nodes().iter().map(|n| n.id().as_str()).collect();
    let expected: Vec<&str> = pipeline.nodes().iter().map(|n| n.id().as_str()).collect();
    assert_eq!(nodes, expected);
    assert_eq!(found.version(), 0);
    assert_eq!(found.status(), JobStatus::Pending);
    assert!(found.started_at().is_none());
    assert!(found.finished_at().is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn status_all_terminal_variants_round_trip(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "st").await;
    let repo = PgJobRepository::new(pool);

    for terminal in [
        JobStatus::Completed,
        JobStatus::Failed,
        JobStatus::Cancelled,
        JobStatus::Orphaned,
    ] {
        let job = JobBuilder::new(&pipeline).terminated(terminal).build();
        repo.create(&job).await.unwrap();
        assert_eq!(repo.find_by_id(job.id()).await.unwrap().status(), terminal);
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_update_writes_the_state_and_moves_to_the_next_version(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "ts").await;
    let repo = PgJobRepository::new(pool);
    let job = job(&pipeline);
    repo.create(&job).await.unwrap();

    let started = repo
        .update(&job.clone().start(clock::now()).unwrap())
        .await
        .unwrap();
    let ended = repo
        .update(&started.clone().complete(clock::now()).unwrap())
        .await
        .unwrap();

    assert_eq!(started.version(), 1);
    assert_eq!(ended.version(), 2);
    let found = repo.find_by_id(job.id()).await.unwrap();
    assert_eq!(found.status(), JobStatus::Completed);
    assert!(found.finished_at().unwrap() >= found.started_at().unwrap());
    assert!(found.updated_at() <= Utc::now());
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_write_of_a_stale_job_is_stale_and_of_a_missing_one_not_found(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "v").await;
    let repo = PgJobRepository::new(pool);
    let job = job(&pipeline);
    repo.create(&job).await.unwrap();
    repo.update(&job.clone().start(clock::now()).unwrap())
        .await
        .unwrap();

    let stale_update = repo
        .update(&job.clone().cancel(clock::now()).unwrap())
        .await;
    let stale_delete = repo.delete(&job).await;
    let ghost = JobBuilder::new(&pipeline).build();
    let missing_update = repo.update(&ghost).await;
    let missing_delete = repo.delete(&ghost).await;

    assert!(matches!(stale_update, Err(DomainError::Stale(_))));
    assert!(matches!(stale_delete, Err(DomainError::Stale(_))));
    assert!(matches!(missing_update, Err(DomainError::NotFound(_))));
    assert!(matches!(missing_delete, Err(DomainError::NotFound(_))));
    assert_eq!(status(&repo, job.id()).await, JobStatus::Running);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_claim_places_the_oldest_visible_job_on_an_idle_agent(pool: PgPool) {
    let (org, project, pipeline) = seed_org_project_pipeline(&pool, "claim").await;
    let agent = seed_agent(&pool, &org, "runner").await;
    let repo = PgJobRepository::new(pool.clone());
    let older = JobBuilder::new(&pipeline)
        .created_at(clock::now() - Duration::seconds(10))
        .build();
    let newer = job(&pipeline);
    repo.create(&newer).await.unwrap();
    repo.create(&older).await.unwrap();

    let stream = stream_of(&agent);

    let elsewhere = Visibility::Scoped {
        orgs: vec![OrganizationId::new("other")],
        projects: vec![ProjectId::new("other")],
    };
    assert!(
        repo.claim_next(&stream, &elsewhere)
            .await
            .unwrap()
            .is_none()
    );

    let own_project = Visibility::Scoped {
        orgs: vec![],
        projects: vec![project.id().clone()],
    };
    let (claimed, project_id) = repo
        .claim_next(&stream, &own_project)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id(), older.id());
    assert_eq!(claimed.agent_app_id(), Some(&agent));
    assert_eq!(claimed.version(), 1);
    assert_eq!(&project_id, project.id());
    assert_eq!(
        repo.pending_streams().await.unwrap(),
        std::slice::from_ref(&stream)
    );

    let own_org = Visibility::Scoped {
        orgs: vec![org.id().clone()],
        projects: vec![],
    };
    assert!(
        repo.claim_next(&stream_of(&agent), &own_org)
            .await
            .unwrap()
            .is_none(),
        "an agent with a job that has not ended gets no other"
    );
    assert!(
        repo.find_by_id(newer.id())
            .await
            .unwrap()
            .agent_app_id()
            .is_none()
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn two_claims_at_once_place_one_job_once(pool: PgPool) {
    let (org, _, pipeline) = seed_org_project_pipeline(&pool, "race").await;
    let first = seed_agent(&pool, &org, "runner-1").await;
    let second = seed_agent(&pool, &org, "runner-2").await;
    let repo = PgJobRepository::new(pool.clone());
    let only = job(&pipeline);
    repo.create(&only).await.unwrap();

    let (first, second) = (stream_of(&first), stream_of(&second));
    let (a, b) = tokio::join!(
        repo.claim_next(&first, &Visibility::All),
        repo.claim_next(&second, &Visibility::All),
    );

    let placed: Vec<Job> = [a.unwrap(), b.unwrap()]
        .into_iter()
        .flatten()
        .map(|(job, _)| job)
        .collect();
    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0].id(), only.id());
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_release_returns_only_the_unstarted_jobs_of_its_stream(pool: PgPool) {
    let (org, _, pipeline) = seed_org_project_pipeline(&pool, "rel").await;
    let agent = seed_agent(&pool, &org, "runner").await;
    let repo = PgJobRepository::new(pool.clone());
    let waiting = job(&pipeline);
    repo.create(&waiting).await.unwrap();
    let (earlier, current) = (stream_of(&agent), stream_of(&agent));
    repo.claim_next(&current, &Visibility::All)
        .await
        .unwrap()
        .unwrap();
    let running = JobBuilder::new(&pipeline)
        .running(true)
        .agent(agent.clone())
        .build();
    repo.create(&running).await.unwrap();

    assert_eq!(repo.release(&earlier).await.unwrap(), 0);
    assert_eq!(
        repo.find_by_id(waiting.id()).await.unwrap().agent_app_id(),
        Some(&agent)
    );
    assert_eq!(repo.release(&current).await.unwrap(), 1);

    let released = repo.find_by_id(waiting.id()).await.unwrap();
    assert!(released.agent_app_id().is_none());
    assert_eq!(released.version(), 2);
    assert!(repo.pending_streams().await.unwrap().is_empty());
    assert_eq!(
        repo.find_by_id(running.id()).await.unwrap().agent_app_id(),
        Some(&agent)
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_live_jobs_are_counted_and_listed_by_agent_and_by_scope(pool: PgPool) {
    let (org, project, pipeline) = seed_org_project_pipeline(&pool, "live").await;
    let (_, _, elsewhere) = seed_org_project_pipeline(&pool, "else").await;
    let agent = seed_agent(&pool, &org, "runner").await;
    let repo = PgJobRepository::new(pool.clone());
    let waiting = JobBuilder::new(&pipeline).agent(agent.clone()).build();
    let running = JobBuilder::new(&pipeline)
        .running(true)
        .agent(agent.clone())
        .build();
    let done = JobBuilder::new(&pipeline)
        .terminated(JobStatus::Completed)
        .agent(agent.clone())
        .build();
    let pooled = job(&pipeline);
    let other = job(&elsewhere);
    for job in [&waiting, &running, &done, &pooled, &other] {
        repo.create(job).await.unwrap();
    }

    assert_eq!(
        repo.active_jobs(&[agent.clone(), AppId::new("idle")])
            .await
            .unwrap(),
        [(agent.clone(), 2)]
    );
    assert_eq!(
        ids(&repo.list_running_on(&agent).await.unwrap()),
        [running.id().clone()]
    );
    let in_pipeline = sorted(vec![
        waiting.id().clone(),
        running.id().clone(),
        pooled.id().clone(),
    ]);
    assert_eq!(
        ids(&repo
            .list_live(JobScope::Pipeline(pipeline.id()))
            .await
            .unwrap()),
        in_pipeline
    );
    assert_eq!(
        ids(&repo
            .list_live(JobScope::Project(project.id()))
            .await
            .unwrap()),
        in_pipeline
    );
    assert_eq!(
        ids(&repo
            .list_live(JobScope::Organization(org.id()))
            .await
            .unwrap()),
        in_pipeline
    );
    assert_eq!(repo.list_live(JobScope::All).await.unwrap().len(), 4);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_running_job_is_stranded_once_its_agent_is_quiet_past_the_cutoff(pool: PgPool) {
    let (org, _, pipeline) = seed_org_project_pipeline(&pool, "reap").await;
    let recent = seed_agent(&pool, &org, "recent").await;
    let quiet = seed_agent(&pool, &org, "quiet").await;
    seen(&pool, &recent, Duration::seconds(5)).await;
    seen(&pool, &quiet, Duration::minutes(10)).await;
    let repo = PgJobRepository::new(pool.clone());
    let running = |agent: &AppId| {
        JobBuilder::new(&pipeline)
            .running(true)
            .agent(agent.clone())
            .build()
    };
    let on_recent = running(&recent);
    let on_quiet = running(&quiet);
    let placed_on_quiet = JobBuilder::new(&pipeline).agent(quiet.clone()).build();
    let agentless = JobBuilder::new(&pipeline).running(true).build();
    let ended = JobBuilder::new(&pipeline)
        .terminated(JobStatus::Completed)
        .agent(quiet.clone())
        .build();
    for job in [&on_recent, &on_quiet, &placed_on_quiet, &agentless, &ended] {
        repo.create(job).await.unwrap();
    }

    let stranded = repo
        .list_stranded(clock::now() - Duration::minutes(1))
        .await
        .unwrap();

    assert_eq!(
        ids(&stranded),
        sorted(vec![on_quiet.id().clone(), agentless.id().clone()])
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_by_pipeline_filters(pool: PgPool) {
    let org = seed_org(&pool, "acme").await;
    let project = seed_project(&pool, &org, "p").await;
    let pipe_repo = PgPipelineRepository::new(pool.clone());
    let pipeline_a = pipeline(&project);
    let pipeline_b = pipeline(&project);
    pipe_repo.create(&pipeline_a).await.unwrap();
    pipe_repo.create(&pipeline_b).await.unwrap();

    let repo = PgJobRepository::new(pool);
    repo.create(&job(&pipeline_a)).await.unwrap();
    repo.create(&job(&pipeline_a)).await.unwrap();
    repo.create(&job(&pipeline_b)).await.unwrap();

    assert_eq!(
        repo.list_by_pipeline(pipeline_a.id(), None)
            .await
            .unwrap()
            .metadata()
            .total_count(),
        2,
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_by_organization_joins_through_pipeline_and_project(pool: PgPool) {
    let (org_target, _, pipeline_target) = seed_org_project_pipeline(&pool, "t").await;
    let (_, _, pipeline_other) = seed_org_project_pipeline(&pool, "o").await;
    let repo = PgJobRepository::new(pool);

    repo.create(&job(&pipeline_target)).await.unwrap();
    repo.create(&job(&pipeline_target)).await.unwrap();
    repo.create(&job(&pipeline_other)).await.unwrap();

    assert_eq!(
        repo.list_by_organization(org_target.id(), None, None)
            .await
            .unwrap()
            .metadata()
            .total_count(),
        2,
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_by_organization_with_an_agent_lists_only_its_jobs_newest_first(pool: PgPool) {
    let (org, _, pipeline) = seed_org_project_pipeline(&pool, "agent").await;
    let (_, _, elsewhere) = seed_org_project_pipeline(&pool, "else").await;
    let agent = seed_agent(&pool, &org, "runner").await;
    let other = seed_agent(&pool, &org, "other").await;
    let repo = PgJobRepository::new(pool.clone());
    let at = |minutes: i64| clock::now() - Duration::minutes(minutes);
    let placed = JobBuilder::new(&pipeline)
        .agent(agent.clone())
        .created_at(at(1))
        .build();
    let running = JobBuilder::new(&pipeline)
        .running(true)
        .agent(agent.clone())
        .created_at(at(2))
        .build();
    let done = JobBuilder::new(&pipeline)
        .terminated(JobStatus::Completed)
        .agent(agent.clone())
        .created_at(at(3))
        .build();
    let on_other = JobBuilder::new(&pipeline)
        .agent(other.clone())
        .created_at(at(4))
        .build();
    let pooled = JobBuilder::new(&pipeline).created_at(at(5)).build();
    let outside = JobBuilder::new(&elsewhere)
        .agent(agent.clone())
        .created_at(at(6))
        .build();
    for job in [&placed, &running, &done, &on_other, &pooled, &outside] {
        repo.create(job).await.unwrap();
    }

    let first = repo
        .list_by_organization(
            org.id(),
            Some(&agent),
            Some(&PaginationParams::new(1, 2).unwrap()),
        )
        .await
        .unwrap();
    let second = repo
        .list_by_organization(
            org.id(),
            Some(&agent),
            Some(&PaginationParams::new(2, 2).unwrap()),
        )
        .await
        .unwrap();
    let all = repo
        .list_by_organization(org.id(), None, None)
        .await
        .unwrap();

    let page = |jobs: &[Job]| jobs.iter().map(|j| j.id().clone()).collect::<Vec<_>>();
    assert_eq!(first.metadata().total_count(), 3);
    assert_eq!(
        page(first.items()),
        [placed.id().clone(), running.id().clone()]
    );
    assert_eq!(page(second.items()), [done.id().clone()]);
    assert_eq!(all.metadata().total_count(), 5);
}

#[sqlx::test(migrations = "../../migrations")]
async fn origin_round_trips_through_jsonb(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "or").await;
    let repo = PgJobRepository::new(pool);

    let origin = JobOrigin::Webhook {
        trigger_id: TriggerId::new("trg-1"),
        delivery_id: Some("gh-42".to_string()),
    };
    let job = JobBuilder::new(&pipeline).origin(origin.clone()).build();
    repo.create(&job).await.unwrap();

    let found = repo.find_by_id(job.id()).await.unwrap();
    assert_eq!(found.origin(), &origin);
}

#[sqlx::test(migrations = "../../migrations")]
async fn cascade_pipeline_delete_removes_jobs(pool: PgPool) {
    let (_, _, pipeline) = seed_org_project_pipeline(&pool, "c").await;
    let repo = PgJobRepository::new(pool.clone());
    let job = job(&pipeline);
    repo.create(&job).await.unwrap();

    PgPipelineRepository::new(pool)
        .delete(&pipeline)
        .await
        .unwrap();

    assert!(matches!(
        repo.find_by_id(job.id()).await,
        Err(DomainError::NotFound(_)),
    ));
}
