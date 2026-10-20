//! The job's reads. One block per query, in the order it runs: the struct, its access, its
//! output type, what `Fetch` reads.

use super::JobUseCases;
use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::domain::errors::DomainResult;
use crate::domain::ids::{AppId, JobId, OrganizationId, PipelineId, ProjectId};
use crate::domain::job::Job;
use crate::domain::permission::Permission;
use async_trait::async_trait;
use scylla_extension::{Access, Authorized, Describe, Fetch, Fetched, Query, Run};

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

#[async_trait]
impl Run<Fetch<GetJob>> for JobUseCases {
    async fn run(&self, input: Authorized<GetJob>) -> DomainResult<Fetched<GetJob>> {
        let job = self.job_repo.find_by_id(&input.command().id).await?;
        Ok(input.fetched(job))
    }
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

#[async_trait]
impl Run<Fetch<ListJobs>> for JobUseCases {
    async fn run(&self, input: Authorized<ListJobs>) -> DomainResult<Fetched<ListJobs>> {
        let page = self
            .job_repo
            .list_all(input.command().pagination.as_ref())
            .await?;
        Ok(input.fetched(page))
    }
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

#[async_trait]
impl Run<Fetch<ListPipelineJobs>> for JobUseCases {
    async fn run(
        &self,
        input: Authorized<ListPipelineJobs>,
    ) -> DomainResult<Fetched<ListPipelineJobs>> {
        let query = input.command();
        let page = self
            .job_repo
            .list_by_pipeline(&query.pipeline_id, query.pagination.as_ref())
            .await?;
        Ok(input.fetched(page))
    }
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

#[async_trait]
impl Run<Fetch<ListProjectJobs>> for JobUseCases {
    async fn run(
        &self,
        input: Authorized<ListProjectJobs>,
    ) -> DomainResult<Fetched<ListProjectJobs>> {
        let query = input.command();
        let page = self
            .job_repo
            .list_by_project(&query.project_id, query.pagination.as_ref())
            .await?;
        Ok(input.fetched(page))
    }
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

#[async_trait]
impl Run<Fetch<ListOrganizationJobs>> for JobUseCases {
    async fn run(
        &self,
        input: Authorized<ListOrganizationJobs>,
    ) -> DomainResult<Fetched<ListOrganizationJobs>> {
        let query = input.command();
        let page = self
            .job_repo
            .list_by_organization(
                &query.organization_id,
                query.agent_id.as_ref(),
                query.pagination.as_ref(),
            )
            .await?;
        Ok(input.fetched(page))
    }
}
