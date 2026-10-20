//! The pipeline's reads. One block per query, in the order it runs: the struct, its access,
//! its output type, what `Fetch` reads.

use super::PipelineUseCases;
use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::domain::errors::DomainResult;
use crate::domain::ids::{OrganizationId, PipelineId, ProjectId};
use crate::domain::permission::Permission;
use crate::domain::pipeline::Pipeline;
use async_trait::async_trait;
use scylla_extension::{Access, Authorized, Describe, Fetch, Fetched, Query, Run};

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

#[async_trait]
impl Run<Fetch<GetPipeline>> for PipelineUseCases {
    async fn run(&self, input: Authorized<GetPipeline>) -> DomainResult<Fetched<GetPipeline>> {
        let pipeline = self.pipeline_repo.find_by_id(&input.command().id).await?;
        Ok(input.fetched(pipeline))
    }
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

#[async_trait]
impl Run<Fetch<ListPipelines>> for PipelineUseCases {
    async fn run(&self, input: Authorized<ListPipelines>) -> DomainResult<Fetched<ListPipelines>> {
        let page = self
            .pipeline_repo
            .list_all(input.command().pagination.as_ref())
            .await?;
        Ok(input.fetched(page))
    }
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

#[async_trait]
impl Run<Fetch<ListProjectPipelines>> for PipelineUseCases {
    async fn run(
        &self,
        input: Authorized<ListProjectPipelines>,
    ) -> DomainResult<Fetched<ListProjectPipelines>> {
        let query = input.command();
        let page = self
            .pipeline_repo
            .list_by_project(&query.project_id, query.pagination.as_ref())
            .await?;
        Ok(input.fetched(page))
    }
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

#[async_trait]
impl Run<Fetch<ListOrganizationPipelines>> for PipelineUseCases {
    async fn run(
        &self,
        input: Authorized<ListOrganizationPipelines>,
    ) -> DomainResult<Fetched<ListOrganizationPipelines>> {
        let query = input.command();
        let page = self
            .pipeline_repo
            .list_by_organization(&query.organization_id, query.pagination.as_ref())
            .await?;
        Ok(input.fetched(page))
    }
}
