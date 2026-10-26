use crate::access::CallerOf;
use crate::ports::{
    Directory, Jobs, MemoryProjects, NoJobs, NoUsers, ProjectStore, Scopes, SeeAll,
};
use scylla_auth::authz::{Grant, PROJECT_ADMIN_ROLE, Principal, Scope, Visibility};
use scylla_core::application::pagination::{PaginatedResult, PaginationParams};
use scylla_domain::domain::caller::CallerContext;
use scylla_domain::domain::errors::DomainResult;
use scylla_domain::domain::ids::{OrganizationId, ProjectId, UserId};
use scylla_domain::domain::permission::Permission;
use scylla_domain::domain::project::{Project, ProjectDescription, ProjectName};
use scylla_domain::domain::role::RoleName;
use scylla_domain::domain::user::User;
use scylla_extension::Access;

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

    #[usecase(write, access = Access::Requires(Permission::CreateProject(organization_id.clone())))]
    async fn create_project(
        &self,
        organization_id: OrganizationId,
        name: ProjectName,
        description: Option<ProjectDescription>,
    ) -> DomainResult<Project> {
        let project = Project::create(name, description, organization_id)?;
        match self.owner_grant(&project)? {
            Some(grant) => {
                self.store().provision_with_owner(&project, &grant).await?;
                Ok(project)
            }
            None => self.store().create(&project).await,
        }
    }

    #[usecase(read, access = Access::Requires(Permission::ReadProject(id.clone())))]
    async fn get_project(&self, id: ProjectId) -> DomainResult<Project> {
        self.store().find_by_id(&id).await
    }

    #[usecase(write, access = Access::Requires(Permission::UpdateProject(id.clone())))]
    async fn update_project(
        &self,
        id: ProjectId,
        name: Option<ProjectName>,
        description: Option<Option<ProjectDescription>>,
    ) -> DomainResult<Project> {
        let mut project = self.store().find_by_id(&id).await?;
        if let Some(name) = name {
            project.update_name(name)?;
        }
        if let Some(description) = description {
            project.update_description(description)?;
        }
        self.store().update(&project).await
    }

    #[usecase(write, access = Access::Requires(Permission::UpdateProject(id.clone())))]
    async fn set_project_active(&self, id: ProjectId, is_active: bool) -> DomainResult<Project> {
        let mut project = self.store().find_by_id(&id).await?;
        project.set_active(is_active);
        self.store().update(&project).await
    }

    #[usecase(write, access = Access::Requires(Permission::DeleteProject(id.clone())))]
    async fn delete_project(&self, id: ProjectId) -> DomainResult<()> {
        let project = self.store().find_by_id(&id).await?;
        let live = self.jobs().live_in_project(project.id()).await?;
        self.store().delete(&project).await?;
        self.jobs().stop(&live).await;
        Ok(())
    }

    #[usecase(read, access = Access::Requires(Permission::ListProjects))]
    async fn list_projects(
        &self,
        pagination: Option<PaginationParams>,
    ) -> DomainResult<PaginatedResult<Project>> {
        self.store().list_all(pagination.as_ref()).await
    }

    /// Gated on `readOrganization`, not on `listProjectsByOrganization`: a project-only role
    /// sees its own projects and is not refused. The wider permission only widens the visible
    /// set. The use case reads it from the grants and does not ask the access model, thus it
    /// writes no audit row.
    #[usecase(read, access = Access::Requires(Permission::ReadOrganization(organization_id.clone())))]
    async fn list_organization_projects(
        &self,
        organization_id: OrganizationId,
        pagination: Option<PaginationParams>,
    ) -> DomainResult<PaginatedResult<Project>> {
        let caller = self.ctx().caller();
        let wide = Permission::ListProjectsByOrganization(organization_id.clone());
        let visible = match self.scopes().visible(caller, wide.key()).await? {
            Visibility::Scoped { orgs, .. } if !orgs.contains(&organization_id) => {
                self.scopes()
                    .visible(caller, Permission::ReadProject(ProjectId::new("_")).key())
                    .await?
            }
            wide => wide,
        };
        self.store()
            .list_by_organization(&organization_id, pagination.as_ref(), &visible)
            .await
    }

    #[usecase(read, access = Access::Requires(Permission::ListProjectMembers(project_id.clone())))]
    async fn list_project_members(
        &self,
        project_id: ProjectId,
        pagination: Option<PaginationParams>,
    ) -> DomainResult<PaginatedResult<User>> {
        let ids = self
            .store()
            .list_principals(&project_id, pagination.as_ref())
            .await?;
        self.directory().users(ids).await
    }

    #[usecase(read, access = Access::Requires(Permission::ListUserProjects(user_id.clone())))]
    async fn list_user_projects(
        &self,
        user_id: UserId,
        pagination: Option<PaginationParams>,
    ) -> DomainResult<PaginatedResult<Project>> {
        let key = Permission::ReadProject(ProjectId::new("_")).key();
        self.store()
            .list_for_user(&user_id, key, pagination.as_ref())
            .await
    }
}
