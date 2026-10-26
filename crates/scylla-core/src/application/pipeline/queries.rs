//! The pipeline's reads. Each query, its access and its output type. The strata module of the slice
//! runs it.

use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::domain::ids::{OrganizationId, PipelineId, ProjectId};
use crate::domain::permission::Permission;
use crate::domain::pipeline::Pipeline;
use scylla_extension::{Access, Describe, Query};

#[derive(Debug)]
pub struct GetPipeline {
    pub id: PipelineId,
}

impl Describe for GetPipeline {
    fn access(&self) -> Access {
        Access::Requires(Permission::ReadPipeline(self.id.clone()))
    }
}

impl Query for GetPipeline {
    type Output = Pipeline;
}

#[derive(Debug)]
pub struct ListPipelines {
    pub pagination: Option<PaginationParams>,
}

impl Describe for ListPipelines {
    fn access(&self) -> Access {
        Access::Requires(Permission::ListPipelines)
    }
}

impl Query for ListPipelines {
    type Output = PaginatedResult<Pipeline>;
}

#[derive(Debug)]
pub struct ListProjectPipelines {
    pub project_id: ProjectId,
    pub pagination: Option<PaginationParams>,
}

impl Describe for ListProjectPipelines {
    fn access(&self) -> Access {
        Access::Requires(Permission::ListPipelinesByProject(self.project_id.clone()))
    }
}

impl Query for ListProjectPipelines {
    type Output = PaginatedResult<Pipeline>;
}

#[derive(Debug)]
pub struct ListOrganizationPipelines {
    pub organization_id: OrganizationId,
    pub pagination: Option<PaginationParams>,
}

impl Describe for ListOrganizationPipelines {
    fn access(&self) -> Access {
        Access::Requires(Permission::ListPipelinesByOrganization(
            self.organization_id.clone(),
        ))
    }
}

impl Query for ListOrganizationPipelines {
    type Output = PaginatedResult<Pipeline>;
}
