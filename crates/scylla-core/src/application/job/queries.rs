//! The job's reads. Each query, its access and its output type. The strata module of the slice runs
//! it.

use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::domain::ids::{AppId, JobId, OrganizationId, PipelineId, ProjectId};
use crate::domain::job::Job;
use crate::domain::permission::Permission;
use scylla_extension::{Access, Describe, Query};

#[derive(Debug)]
pub struct GetJob {
    pub id: JobId,
}

impl Describe for GetJob {
    fn access(&self) -> Access {
        Access::Requires(Permission::ReadJob(self.id.clone()))
    }
}

impl Query for GetJob {
    type Output = Job;
}

#[derive(Debug)]
pub struct ListJobs {
    pub pagination: Option<PaginationParams>,
}

impl Describe for ListJobs {
    fn access(&self) -> Access {
        Access::Requires(Permission::ListJobs)
    }
}

impl Query for ListJobs {
    type Output = PaginatedResult<Job>;
}

#[derive(Debug)]
pub struct ListPipelineJobs {
    pub pipeline_id: PipelineId,
    pub pagination: Option<PaginationParams>,
}

impl Describe for ListPipelineJobs {
    fn access(&self) -> Access {
        Access::Requires(Permission::ListJobsByPipeline(self.pipeline_id.clone()))
    }
}

impl Query for ListPipelineJobs {
    type Output = PaginatedResult<Job>;
}

#[derive(Debug)]
pub struct ListProjectJobs {
    pub project_id: ProjectId,
    pub pagination: Option<PaginationParams>,
}

impl Describe for ListProjectJobs {
    fn access(&self) -> Access {
        Access::Requires(Permission::ListJobsByProject(self.project_id.clone()))
    }
}

impl Query for ListProjectJobs {
    type Output = PaginatedResult<Job>;
}

/// With `agent_id`, only the jobs placed on that agent.
#[derive(Debug)]
pub struct ListOrganizationJobs {
    pub organization_id: OrganizationId,
    pub agent_id: Option<AppId>,
    pub pagination: Option<PaginationParams>,
}

impl Describe for ListOrganizationJobs {
    fn access(&self) -> Access {
        Access::Requires(Permission::ListJobsByOrganization(
            self.organization_id.clone(),
        ))
    }
}

impl Query for ListOrganizationJobs {
    type Output = PaginatedResult<Job>;
}
