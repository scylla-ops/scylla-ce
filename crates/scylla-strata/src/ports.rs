use scylla_auth::authz::{Grant, Principal, Visibility};
use scylla_core::application::job::JobScope;
use scylla_core::application::pagination::{PaginatedResult, PaginationParams};
use scylla_core::application::user::{UserRepository, users_in_order};
use scylla_core::application::{DispatchUseCases, JobRepository, ProjectRepository};
use scylla_domain::domain::caller::CallerContext;
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{OrganizationId, ProjectId, UserId};
use scylla_domain::domain::job::Job;
use scylla_domain::domain::project::Project;
use scylla_domain::domain::user::User;
use std::sync::{Arc, Mutex};
use strata::{BuildCx, BuildError, Component, Resource};

#[strata::port]
pub trait ProjectStore: Send + Sync + 'static {
    async fn create(&self, project: &Project) -> DomainResult<Project>;
    async fn provision_with_owner(&self, project: &Project, grant: &Grant) -> DomainResult<()>;
    async fn find_by_id(&self, id: &ProjectId) -> DomainResult<Project>;
    async fn update(&self, project: &Project) -> DomainResult<Project>;
    async fn delete(&self, project: &Project) -> DomainResult<()>;
    async fn list_all(
        &self,
        page: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Project>>;
    async fn list_by_organization(
        &self,
        organization: &OrganizationId,
        page: Option<&PaginationParams>,
        visible: &Visibility,
    ) -> DomainResult<PaginatedResult<Project>>;
    async fn list_principals(
        &self,
        id: &ProjectId,
        page: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<UserId>>;
    async fn list_for_user(
        &self,
        user: &UserId,
        permission: &str,
        page: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Project>>;
}

#[strata::port]
pub trait Scopes: Send + Sync + 'static {
    async fn visible(&self, caller: &CallerContext, permission: &str) -> DomainResult<Visibility>;
}

#[strata::port]
pub trait Directory: Send + Sync + 'static {
    async fn users(&self, ids: PaginatedResult<UserId>) -> DomainResult<PaginatedResult<User>>;
}

#[strata::port]
pub trait Jobs: Send + Sync + 'static {
    async fn live_in_project(&self, id: &ProjectId) -> DomainResult<Vec<Job>>;
    async fn stop(&self, jobs: &[Job]);
}

fn page_of<T: Clone>(rows: &[T], page: Option<&PaginationParams>) -> PaginatedResult<T> {
    let page = page.copied().unwrap_or_default();
    let at = |n: u64| usize::try_from(n).unwrap_or(usize::MAX);
    let items = rows
        .iter()
        .skip(at(page.offset()))
        .take(at(page.limit()))
        .cloned()
        .collect();
    PaginatedResult::new(items, &page, rows.len() as u64)
}

/// The projects and the project grants, in memory.
#[derive(strata::Component)]
#[dev = "keeps the projects in memory"]
pub struct MemoryProjects {
    rows: Mutex<Vec<Project>>,
    owners: Mutex<Vec<(ProjectId, UserId)>>,
}

impl MemoryProjects {
    fn stale_or_missing(&self, project: &Project) -> DomainResult<usize> {
        let rows = self.rows.lock().unwrap();
        let i = rows
            .iter()
            .position(|p| p.id() == project.id())
            .ok_or_else(|| DomainError::not_found("Project", project.id()))?;
        if rows[i].version() != project.version() {
            return Err(DomainError::stale("Project", project.id()));
        }
        Ok(i)
    }
}

fn bumped(project: &Project) -> Project {
    Project::from_persistence(
        project.id().clone(),
        project.name().clone(),
        project.description().cloned(),
        project.organization_id().clone(),
        project.is_active(),
        project.created_at(),
        project.updated_at(),
        project.version() + 1,
    )
}

impl ProjectStore for MemoryProjects {
    async fn create(&self, project: &Project) -> DomainResult<Project> {
        self.rows.lock().unwrap().push(project.clone());
        Ok(project.clone())
    }

    async fn provision_with_owner(&self, project: &Project, grant: &Grant) -> DomainResult<()> {
        self.create(project).await?;
        if let Principal::User(user) = &grant.principal {
            self.owners
                .lock()
                .unwrap()
                .push((project.id().clone(), user.clone()));
        }
        Ok(())
    }

    async fn find_by_id(&self, id: &ProjectId) -> DomainResult<Project> {
        let rows = self.rows.lock().unwrap();
        rows.iter()
            .find(|p| p.id() == id)
            .cloned()
            .ok_or_else(|| DomainError::not_found("Project", id))
    }

    async fn update(&self, project: &Project) -> DomainResult<Project> {
        let i = self.stale_or_missing(project)?;
        let row = bumped(project);
        self.rows.lock().unwrap()[i] = row.clone();
        Ok(row)
    }

    async fn delete(&self, project: &Project) -> DomainResult<()> {
        let i = self.stale_or_missing(project)?;
        self.rows.lock().unwrap().remove(i);
        self.owners
            .lock()
            .unwrap()
            .retain(|(p, _)| p != project.id());
        Ok(())
    }

    async fn list_all(
        &self,
        page: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Project>> {
        Ok(page_of(&self.rows.lock().unwrap(), page))
    }

    async fn list_by_organization(
        &self,
        organization: &OrganizationId,
        page: Option<&PaginationParams>,
        visible: &Visibility,
    ) -> DomainResult<PaginatedResult<Project>> {
        let rows: Vec<Project> = self
            .rows
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.organization_id() == organization)
            .filter(|p| match visible {
                Visibility::All => true,
                Visibility::Scoped { orgs, projects } => {
                    orgs.contains(organization) || projects.contains(p.id())
                }
            })
            .cloned()
            .collect();
        Ok(page_of(&rows, page))
    }

    async fn list_principals(
        &self,
        id: &ProjectId,
        page: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<UserId>> {
        let users: Vec<UserId> = self
            .owners
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p == id)
            .map(|(_, u)| u.clone())
            .collect();
        Ok(page_of(&users, page))
    }

    async fn list_for_user(
        &self,
        user: &UserId,
        _permission: &str,
        page: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Project>> {
        let owned: Vec<ProjectId> = self
            .owners
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, u)| u == user)
            .map(|(p, _)| p.clone())
            .collect();
        let rows: Vec<Project> = self
            .rows
            .lock()
            .unwrap()
            .iter()
            .filter(|p| owned.contains(p.id()))
            .cloned()
            .collect();
        Ok(page_of(&rows, page))
    }
}

/// Sees every scope.
#[derive(strata::Component)]
#[dev = "shows every project to every caller"]
pub struct SeeAll;

impl Scopes for SeeAll {
    async fn visible(&self, _: &CallerContext, _: &str) -> DomainResult<Visibility> {
        Ok(Visibility::All)
    }
}

/// Knows no user.
#[derive(strata::Component)]
#[dev = "knows no user"]
pub struct NoUsers;

impl Directory for NoUsers {
    async fn users(&self, ids: PaginatedResult<UserId>) -> DomainResult<PaginatedResult<User>> {
        let (_, metadata) = ids.into_parts();
        Ok(PaginatedResult::from_parts(Vec::new(), metadata))
    }
}

/// Runs no job.
#[derive(strata::Component)]
#[dev = "starts no job"]
pub struct NoJobs;

impl Jobs for NoJobs {
    async fn live_in_project(&self, _: &ProjectId) -> DomainResult<Vec<Job>> {
        Ok(Vec::new())
    }

    async fn stop(&self, _: &[Job]) {}
}

#[derive(Clone)]
pub struct ScyllaRepos {
    pub db: sqlx::PgPool,
    pub projects: Arc<dyn ProjectRepository>,
    pub users: Arc<dyn UserRepository>,
    pub jobs: Arc<dyn JobRepository>,
}

impl Resource for ScyllaRepos {
    const SECTION: &'static str = "database";
    async fn build(_: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Err(BuildError::msg(
            "the repositories are built by the server: give them with `Builder::provide`",
        ))
    }
}

pub struct PgProjects(Arc<dyn ProjectRepository>);

impl Component for PgProjects {
    async fn build(cx: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Ok(PgProjects(cx.resource::<ScyllaRepos>().await?.projects))
    }
}

impl ProjectStore for PgProjects {
    async fn create(&self, project: &Project) -> DomainResult<Project> {
        self.0.create(project).await
    }
    async fn provision_with_owner(&self, project: &Project, grant: &Grant) -> DomainResult<()> {
        self.0.provision_with_owner(project, grant).await
    }
    async fn find_by_id(&self, id: &ProjectId) -> DomainResult<Project> {
        self.0.find_by_id(id).await
    }
    async fn update(&self, project: &Project) -> DomainResult<Project> {
        self.0.update(project).await
    }
    async fn delete(&self, project: &Project) -> DomainResult<()> {
        self.0.delete(project).await
    }
    async fn list_all(
        &self,
        page: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Project>> {
        self.0.list_all(page).await
    }
    async fn list_by_organization(
        &self,
        organization: &OrganizationId,
        page: Option<&PaginationParams>,
        visible: &Visibility,
    ) -> DomainResult<PaginatedResult<Project>> {
        self.0
            .list_by_organization(organization, page, visible)
            .await
    }
    async fn list_principals(
        &self,
        id: &ProjectId,
        page: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<UserId>> {
        self.0.list_principals(id, page).await
    }
    async fn list_for_user(
        &self,
        user: &UserId,
        permission: &str,
        page: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Project>> {
        self.0.list_for_user(user, permission, page).await
    }
}

pub struct PgDirectory(Arc<dyn UserRepository>);

impl Component for PgDirectory {
    async fn build(cx: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Ok(PgDirectory(cx.resource::<ScyllaRepos>().await?.users))
    }
}

impl Directory for PgDirectory {
    async fn users(&self, ids: PaginatedResult<UserId>) -> DomainResult<PaginatedResult<User>> {
        users_in_order(self.0.as_ref(), ids).await
    }
}

#[derive(Clone)]
pub struct JobDispatch(pub Arc<DispatchUseCases>);

impl Resource for JobDispatch {
    const SECTION: &'static str = "dispatch";
    async fn build(_: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Err(BuildError::msg(
            "the dispatcher is built by the server: give it with `Builder::provide`",
        ))
    }
}

pub struct DispatchJobs {
    jobs: Arc<dyn JobRepository>,
    dispatch: Arc<DispatchUseCases>,
}

impl Component for DispatchJobs {
    async fn build(cx: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Ok(DispatchJobs {
            jobs: cx.resource::<ScyllaRepos>().await?.jobs,
            dispatch: cx.resource::<JobDispatch>().await?.0,
        })
    }
}

impl Jobs for DispatchJobs {
    async fn live_in_project(&self, id: &ProjectId) -> DomainResult<Vec<Job>> {
        self.jobs.list_live(JobScope::Project(id)).await
    }

    async fn stop(&self, jobs: &[Job]) {
        for job in jobs {
            self.dispatch.stop(job);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DispatchJobs, Jobs};
    use scylla_core::test_support::jobs::JobBuilder;
    use scylla_core::test_support::pipelines::PipelineBuilder;
    use scylla_core::test_support::stubs::{StubJobs, StubRegistry, StubTails, dispatcher};
    use scylla_domain::domain::ids::{AppId, ProjectId};
    use scylla_domain::domain::job::JobStatus;
    use std::sync::Arc;

    #[tokio::test]
    async fn the_jobs_port_stops_each_live_job_of_the_project_on_its_agent() {
        let project = ProjectId::new("proj-1");
        let pipeline = PipelineBuilder::for_project_id(project.clone()).build();
        let agent = AppId::new("agent-1");
        let registry = Arc::new(StubRegistry::default());
        registry.connect(&agent);
        let jobs = Arc::new(StubJobs::default());
        let running = jobs.insert(
            &JobBuilder::new(&pipeline)
                .running(true)
                .agent(agent.clone())
                .build(),
        );
        jobs.insert(
            &JobBuilder::new(&pipeline)
                .terminated(JobStatus::Completed)
                .agent(agent.clone())
                .build(),
        );
        let tails = Arc::new(StubTails::default());
        let port = DispatchJobs {
            jobs: jobs.clone(),
            dispatch: dispatcher(registry.clone(), jobs, tails.clone()),
        };

        let live = port.live_in_project(&project).await.unwrap();
        port.stop(&live).await;

        assert_eq!(live.len(), 1);
        assert_eq!(registry.cancelled(), [(agent, running.id().clone())]);
        assert_eq!(tails.closed(), [running.id().clone()]);
    }
}
