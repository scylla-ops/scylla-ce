use crate::domain::ids::JobId;
use crate::domain::ids::UserId;
use crate::domain::job::Job;
use crate::domain::job::JobLog;
use crate::domain::organization::Organization;
use crate::domain::pipeline::Pipeline;
use crate::domain::project::Project;
use crate::domain::session::Session;
use crate::domain::user::User;
use scylla_core::test_support::prelude::*;

pub async fn seed_org(pool: &sqlx::PgPool, name: &str) -> Organization {
    use crate::postgres::PgOrganizationRepository;
    use scylla_core::application::OrganizationRepository;
    let org = org(name);
    PgOrganizationRepository::new(pool.clone())
        .create(&org)
        .await
        .expect("seed org failed");
    org
}

pub async fn seed_project(pool: &sqlx::PgPool, org: &Organization, name: &str) -> Project {
    use crate::postgres::PgProjectRepository;
    use scylla_core::application::ProjectRepository;
    let project = project(org, name);
    PgProjectRepository::new(pool.clone())
        .create(&project)
        .await
        .expect("seed project failed");
    project
}

pub async fn seed_pipeline(pool: &sqlx::PgPool, project: &Project) -> Pipeline {
    use crate::postgres::PgPipelineRepository;
    use scylla_core::application::PipelineRepository;
    let pipeline = pipeline(project);
    PgPipelineRepository::new(pool.clone())
        .create(&pipeline)
        .await
        .expect("seed pipeline failed");
    pipeline
}

pub async fn seed_job(pool: &sqlx::PgPool, pipeline: &Pipeline) -> Job {
    use crate::postgres::PgJobRepository;
    use scylla_core::application::JobRepository;
    let job = job(pipeline);
    PgJobRepository::new(pool.clone())
        .create(&job)
        .await
        .expect("seed job failed");
    job
}

pub async fn seed_job_log(
    pool: &sqlx::PgPool,
    job_id: &JobId,
    node_id: &str,
    line: &str,
) -> JobLog {
    use crate::postgres::PgJobLogRepository;
    use scylla_core::application::JobLogRepository;
    let log = job_log(job_id, node_id, line);
    PgJobLogRepository::new(pool.clone())
        .create_many(std::slice::from_ref(&log))
        .await
        .expect("seed job log failed");
    log
}

pub async fn seed_session(pool: &sqlx::PgPool, user_id: &UserId) -> Session {
    use crate::postgres::PgSessionRepository;
    use scylla_core::application::SessionRepository;
    let session = session(user_id);
    PgSessionRepository::new(pool.clone())
        .create(&session)
        .await
        .expect("seed session failed");
    session
}

pub async fn seed_user(pool: &sqlx::PgPool, name: &str) -> User {
    use crate::postgres::PgUserRepository;
    use scylla_core::application::UserRepository;
    let user = user(name);
    PgUserRepository::new(pool.clone())
        .create(&user)
        .await
        .expect("seed user failed");
    user
}
