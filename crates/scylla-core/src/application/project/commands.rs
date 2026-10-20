//! The project's writes. One block per command, in the order it runs: the struct, its
//! access and path, its payload types, what `Prepare` builds, what `Persist` writes.

use super::ProjectUseCases;
use crate::application::job::JobScope;
use crate::domain::caller::CallerContext;
use crate::domain::errors::DomainResult;
use crate::domain::ids::{OrganizationId, ProjectId};
use crate::domain::permission::Permission;
use crate::domain::project::{Project, ProjectDescription, ProjectName};
use crate::domain::role::RoleName;
use async_trait::async_trait;
use scylla_auth::authz::{Grant, PROJECT_ADMIN_ROLE, Principal, Scope};
use scylla_extension::{
    Access, Authorized, Command, Committed, Deleted, Describe, Draft, Persist, Prepare, Prepared,
    Run,
};

#[derive(Debug)]
pub struct CreateProject {
    pub organization_id: OrganizationId,
    pub name: ProjectName,
    pub description: Option<ProjectDescription>,
}

/// For a user caller, the grant that makes them the project's admin; both rows go in one
/// transaction, so they are staged together.
#[derive(Debug)]
pub struct NewProject {
    pub project: Project,
    pub owner: Option<Grant>,
}

impl Describe for CreateProject {
    fn access(&self) -> Access {
        Access::Requires(Permission::CreateProject(self.organization_id.clone()))
    }
}

impl Command for CreateProject {
    type Staged = Draft<NewProject>;
    type Committed = Project;
}

#[async_trait]
impl Run<Prepare<CreateProject>> for ProjectUseCases {
    async fn run(&self, input: Authorized<CreateProject>) -> DomainResult<Prepared<CreateProject>> {
        let cmd = input.command();
        let project = Project::create(
            cmd.name.clone(),
            cmd.description.clone(),
            cmd.organization_id.clone(),
        )?;
        let owner = match input.caller() {
            CallerContext::User(user_id) => Some(Grant::new(
                Principal::User(user_id.clone()),
                RoleName::new(PROJECT_ADMIN_ROLE)?,
                Scope::Project(project.id().clone()),
            )),
            _ => None,
        };
        Ok(input.prepared(Draft::new(NewProject { project, owner })))
    }
}

#[async_trait]
impl Run<Persist<CreateProject>> for ProjectUseCases {
    async fn run(&self, input: Prepared<CreateProject>) -> DomainResult<Committed<CreateProject>> {
        input
            .commit(async |draft| {
                let NewProject { project, owner } = draft.into_inner();
                match owner {
                    Some(grant) => {
                        self.project_repo
                            .provision_with_owner(&project, &grant)
                            .await?;
                        Ok(project)
                    }
                    None => self.project_repo.create(&project).await,
                }
            })
            .await
    }
}

#[derive(Debug)]
pub struct UpdateProject {
    pub id: ProjectId,
    pub name: Option<ProjectName>,
    pub description: Option<Option<ProjectDescription>>,
}

impl Describe for UpdateProject {
    fn access(&self) -> Access {
        Access::Requires(Permission::UpdateProject(self.id.clone()))
    }
}

impl Command for UpdateProject {
    type Staged = Draft<Project>;
    type Committed = Project;
}

#[async_trait]
impl Run<Prepare<UpdateProject>> for ProjectUseCases {
    async fn run(&self, input: Authorized<UpdateProject>) -> DomainResult<Prepared<UpdateProject>> {
        let cmd = input.command();
        let mut project = self.project_repo.find_by_id(&cmd.id).await?;
        if let Some(name) = &cmd.name {
            project.update_name(name.clone())?;
        }
        if let Some(description) = &cmd.description {
            project.update_description(description.clone())?;
        }
        Ok(input.prepared(Draft::new(project)))
    }
}

#[async_trait]
impl Run<Persist<UpdateProject>> for ProjectUseCases {
    async fn run(&self, input: Prepared<UpdateProject>) -> DomainResult<Committed<UpdateProject>> {
        input
            .commit(async |draft| self.project_repo.update(&draft.into_inner()).await)
            .await
    }
}

#[derive(Debug)]
pub struct SetProjectActive {
    pub id: ProjectId,
    pub is_active: bool,
}

impl Describe for SetProjectActive {
    fn access(&self) -> Access {
        Access::Requires(Permission::UpdateProject(self.id.clone()))
    }
}

impl Command for SetProjectActive {
    type Staged = Draft<Project>;
    type Committed = Project;
}

#[async_trait]
impl Run<Prepare<SetProjectActive>> for ProjectUseCases {
    async fn run(
        &self,
        input: Authorized<SetProjectActive>,
    ) -> DomainResult<Prepared<SetProjectActive>> {
        let cmd = input.command();
        let mut project = self.project_repo.find_by_id(&cmd.id).await?;
        project.set_active(cmd.is_active);
        Ok(input.prepared(Draft::new(project)))
    }
}

#[async_trait]
impl Run<Persist<SetProjectActive>> for ProjectUseCases {
    async fn run(
        &self,
        input: Prepared<SetProjectActive>,
    ) -> DomainResult<Committed<SetProjectActive>> {
        input
            .commit(async |draft| self.project_repo.update(&draft.into_inner()).await)
            .await
    }
}

#[derive(Debug)]
pub struct DeleteProject {
    pub id: ProjectId,
}

impl Describe for DeleteProject {
    fn access(&self) -> Access {
        Access::Requires(Permission::DeleteProject(self.id.clone()))
    }
}

impl Command for DeleteProject {
    type Staged = Project;
    type Committed = Deleted<Project>;
}

#[async_trait]
impl Run<Prepare<DeleteProject>> for ProjectUseCases {
    async fn run(&self, input: Authorized<DeleteProject>) -> DomainResult<Prepared<DeleteProject>> {
        let project = self.project_repo.find_by_id(&input.command().id).await?;
        Ok(input.prepared(project))
    }
}

#[async_trait]
impl Run<Persist<DeleteProject>> for ProjectUseCases {
    async fn run(&self, input: Prepared<DeleteProject>) -> DomainResult<Committed<DeleteProject>> {
        input
            .commit(async |project| {
                self.dispatch
                    .recall(
                        JobScope::Project(project.id()),
                        self.project_repo.delete(&project),
                    )
                    .await?;
                Ok(Deleted::new(project))
            })
            .await
    }
}
