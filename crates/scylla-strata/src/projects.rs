use crate::access::CallerOf;
use crate::ports::{
    Directory, Jobs, MemoryProjects, NoJobs, NoUsers, ProjectStore, Scopes, SeeAll,
};
use scylla_auth::authz::{Grant, PROJECT_ADMIN_ROLE, Principal, Scope, Visibility};
use scylla_core::application::pagination::PaginatedResult;
use scylla_core::application::project::{
    CreateProject, DeleteProject, GetProject, ListOrganizationProjects, ListProjectMembers,
    ListProjects, ListUserProjects, SetProjectActive, UpdateProject,
};
use scylla_domain::domain::caller::CallerContext;
use scylla_domain::domain::errors::DomainResult;
use scylla_domain::domain::ids::ProjectId;
use scylla_domain::domain::permission::Permission;
use scylla_domain::domain::project::Project;
use scylla_domain::domain::role::RoleName;
use scylla_domain::domain::user::User;
use scylla_extension::Describe;

/// The projects of an organization. Each use case checks its permission first, as the
/// authorize stage of Scylla does, then reads or writes through the store.
#[strata::module(section = "projects")]
pub trait Projects: crate::Access {
    #[component]
    type Store: ProjectStore = MemoryProjects;

    #[component]
    type Scopes: Scopes = SeeAll;

    #[component]
    type Directory: Directory = NoUsers;

    #[component]
    type Jobs: Jobs = NoJobs;

    /// For a user caller, the grant that makes them the admin of the new project.
    fn owner_grant(&self, project: &Project) -> DomainResult<Option<Grant>> {
        match self.ctx().caller() {
            CallerContext::User(user) => Ok(Some(Grant::new(
                Principal::User(user.clone()),
                RoleName::new(PROJECT_ADMIN_ROLE)?,
                Scope::Project(project.id().clone()),
            ))),
            _ => Ok(None),
        }
    }

    #[usecase(write, access = cmd.access())]
    async fn create_project(&self, cmd: CreateProject) -> DomainResult<Project> {
        let project = Project::create(cmd.name, cmd.description, cmd.organization_id)?;
        match self.owner_grant(&project)? {
            Some(grant) => {
                self.store().provision_with_owner(&project, &grant).await?;
                Ok(project)
            }
            None => self.store().create(&project).await,
        }
    }

    #[usecase(read, access = query.access())]
    async fn get_project(&self, query: GetProject) -> DomainResult<Project> {
        self.store().find_by_id(&query.id).await
    }

    #[usecase(write, access = cmd.access())]
    async fn update_project(&self, cmd: UpdateProject) -> DomainResult<Project> {
        let mut project = self.store().find_by_id(&cmd.id).await?;
        if let Some(name) = cmd.name {
            project.update_name(name)?;
        }
        if let Some(description) = cmd.description {
            project.update_description(description)?;
        }
        self.store().update(&project).await
    }

    #[usecase(write, access = cmd.access())]
    async fn set_project_active(&self, cmd: SetProjectActive) -> DomainResult<Project> {
        let mut project = self.store().find_by_id(&cmd.id).await?;
        project.set_active(cmd.is_active);
        self.store().update(&project).await
    }

    #[usecase(write, access = cmd.access())]
    async fn delete_project(&self, cmd: DeleteProject) -> DomainResult<()> {
        let project = self.store().find_by_id(&cmd.id).await?;
        let live = self.jobs().live_in_project(project.id()).await?;
        self.store().delete(&project).await?;
        self.jobs().stop(&live).await;
        Ok(())
    }

    #[usecase(read, access = query.access())]
    async fn list_projects(&self, query: ListProjects) -> DomainResult<PaginatedResult<Project>> {
        self.store().list_all(query.pagination.as_ref()).await
    }

    /// Gated on `readOrganization`: a project-only role sees its own projects. The wider
    /// permission only widens the visible set.
    #[usecase(read, access = query.access())]
    async fn list_organization_projects(
        &self,
        query: ListOrganizationProjects,
    ) -> DomainResult<PaginatedResult<Project>> {
        let caller = self.ctx().caller();
        let wide = Permission::ListProjectsByOrganization(query.organization_id.clone());
        let visible = match self.scopes().visible(caller, wide.key()).await? {
            Visibility::Scoped { orgs, .. } if !orgs.contains(&query.organization_id) => {
                self.scopes()
                    .visible(caller, Permission::ReadProject(ProjectId::new("_")).key())
                    .await?
            }
            wide => wide,
        };
        self.store()
            .list_by_organization(&query.organization_id, query.pagination.as_ref(), &visible)
            .await
    }

    #[usecase(read, access = query.access())]
    async fn list_project_members(
        &self,
        query: ListProjectMembers,
    ) -> DomainResult<PaginatedResult<User>> {
        let ids = self
            .store()
            .list_principals(&query.project_id, query.pagination.as_ref())
            .await?;
        self.directory().users(ids).await
    }

    #[usecase(read, access = query.access())]
    async fn list_user_projects(
        &self,
        query: ListUserProjects,
    ) -> DomainResult<PaginatedResult<Project>> {
        let key = Permission::ReadProject(ProjectId::new("_")).key();
        self.store()
            .list_for_user(&query.user_id, key, query.pagination.as_ref())
            .await
    }
}
