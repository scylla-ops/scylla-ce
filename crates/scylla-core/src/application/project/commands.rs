//! The project's writes. Each command, its access and its payload types. The strata module of the
//! slice runs it.

use crate::domain::ids::{OrganizationId, ProjectId};
use crate::domain::permission::Permission;
use crate::domain::project::{Project, ProjectDescription, ProjectName};
use scylla_auth::authz::Grant;
use scylla_extension::{Access, Command, Deleted, Describe, Draft};

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
