//! Unit tests of the jobs of the agents that the API cannot show: the lost agents, which take 60
//! seconds, and the check of the jobs after a new stream.

use std::time::Duration;

use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::apps::{AppId, NewApp};
use scylla_domain::identity::users::{Email, NewUser};
use scylla_domain::projects::jobs::{JobEvent, JobId, JobStatus, NewJob};
use scylla_domain::projects::pipelines::{Node, NodeId, Runtime, Step};
use scylla_domain::projects::{NewProject, ProjectDescription, ProjectName};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::Name;
use scylla_extension::Hooks;
use sqlx::PgPool;

use super::{housekeeping, runs, service};
use crate::features::identity::agents::Registry;
use crate::features::identity::{accounts, apps};
use crate::features::projects::service as projects;

/// The place of a test: a user, its workspace, an agent, and a pipeline of a project.
struct Place {
    /// The user.
    user: Actor,
    /// The workspace of the user.
    workspace: WorkspaceId,
    /// The agent.
    agent: AppId,
    /// The pipeline.
    pipeline: scylla_domain::projects::pipelines::PipelineId,
    /// The project.
    project: scylla_domain::projects::ProjectId,
}

/// Creates a user, a project with a pipeline row, and an agent in the workspace of the user.
async fn place_of(pool: &PgPool) -> Place {
    let new = NewUser {
        email: Email::parse(format!("{}@test.example", ActorId::generate())).unwrap(),
        email_verified: true,
        display_name: Name::parse("Test".to_owned()).unwrap(),
        handle: None,
        password: None,
    };
    let user = accounts::create_user(pool, new).await.unwrap();
    let workspace = WorkspaceId::from_db(
        sqlx::query_scalar!(
            "SELECT workspace_id FROM workspace_owners WHERE user_id = $1",
            user.id.to_db()
        )
        .fetch_one(pool)
        .await
        .unwrap(),
    );
    let new = NewProject {
        workspace_id: workspace,
        created_by: user.id,
        name: ProjectName::parse("alpha".to_owned()).unwrap(),
        description: ProjectDescription::default(),
    };
    let project = projects::create(pool, &Hooks::default(), user.clone(), new)
        .await
        .unwrap()
        .id;
    let pipeline = scylla_domain::projects::pipelines::PipelineId::generate();
    sqlx::query!(
        "INSERT INTO pipelines (id, project_id, workspace_id, created_by, name, nodes)
         VALUES ($1, $2, $3, $4, 'build', '[]')",
        pipeline.to_db(),
        project.to_db(),
        workspace.to_db(),
        user.id.to_db(),
    )
    .execute(pool)
    .await
    .unwrap();
    let new = NewApp {
        workspace_id: workspace,
        created_by: user.id,
        name: Name::parse("runner".to_owned()).unwrap(),
        runner: true,
    };
    let (agent, _) = apps::create(pool, new).await.unwrap();
    Place {
        user,
        workspace,
        agent: agent.id,
        pipeline,
        project,
    }
}

/// Creates a user, a project and a pipeline row with no node. Returns the user, the workspace,
/// the project and the pipeline.
pub(crate) async fn place(
    pool: &PgPool,
) -> (
    Actor,
    WorkspaceId,
    scylla_domain::projects::ProjectId,
    scylla_domain::projects::pipelines::PipelineId,
) {
    let place = place_of(pool).await;
    (place.user, place.workspace, place.project, place.pipeline)
}

/// Starts a job of one node and gives it to the agent.
async fn placed_job(pool: &PgPool, place: &Place) -> JobId {
    let node = Node {
        id: NodeId::parse("build".to_owned()).unwrap(),
        needs: Vec::new(),
        step: Step::Exec(vec!["make".to_owned()]),
        env: Vec::new(),
        timeout_seconds: 60,
        retries: 0,
        working_dir: None,
        runtime: Runtime::Process,
    };
    let new = NewJob {
        pipeline_id: place.pipeline,
        project_id: place.project,
        workspace_id: place.workspace,
        created_by: place.user.id,
        rerun_of: None,
        trigger_id: None,
        nodes: vec![node],
    };
    let registry = Registry::default();
    let job = service::create(pool, &Hooks::default(), &registry, place.user.clone(), new)
        .await
        .unwrap();
    let claimed = runs::claim(pool, None, place.workspace, None, place.agent, false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.job.id, job.id);
    job.id
}

/// Returns the state and the agent of a job.
async fn state(pool: &PgPool, id: JobId) -> (JobStatus, Option<AppId>) {
    let job = service::get(pool, id).await.unwrap().unwrap();
    (job.status, job.agent_id)
}

/// A lost agent loses its jobs: a running job ends as orphaned, and a placed job goes back to
/// the pool. An agent with an open stream keeps them.
#[sqlx::test(migrator = "crate::infra::db::MIGRATOR")]
async fn reap_releases_the_jobs_of_a_lost_agent(pool: PgPool) {
    // given
    let place = place_of(&pool).await;
    let running = placed_job(&pool, &place).await;
    runs::apply(&pool, place.agent, running, &JobEvent::JobStarted)
        .await
        .unwrap();
    let waiting = placed_job(&pool, &place).await;
    let connected = Registry::default();
    let _ticket = connected.open(place.agent);

    // when
    let kept = housekeeping::reap(&pool, &connected, Duration::ZERO)
        .await
        .unwrap();
    let reaped = housekeeping::reap(&pool, &Registry::default(), Duration::ZERO)
        .await
        .unwrap();

    // then
    assert_eq!((kept, reaped), (0, 2));
    assert_eq!(state(&pool, running).await.0, JobStatus::Orphaned);
    assert_eq!(state(&pool, waiting).await, (JobStatus::Pending, None));
}

/// After a new stream, the jobs that the agent does not list are released, and a listed job
/// that the agent does not hold must stop.
#[sqlx::test(migrator = "crate::infra::db::MIGRATOR")]
async fn reconcile_checks_the_jobs_of_a_new_stream(pool: PgPool) {
    // given
    let place = place_of(&pool).await;
    let kept = placed_job(&pool, &place).await;
    let lost = placed_job(&pool, &place).await;
    runs::apply(&pool, place.agent, lost, &JobEvent::JobStarted)
        .await
        .unwrap();
    let stranger = JobId::generate();

    // when
    let (stop, pooled) = runs::reconcile(&pool, place.agent, &[kept, stranger])
        .await
        .unwrap();

    // then
    assert_eq!(stop, vec![stranger]);
    assert!(!pooled);
    assert_eq!(state(&pool, kept).await.1, Some(place.agent));
    assert_eq!(state(&pool, lost).await.0, JobStatus::Orphaned);
}
