use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::domain::errors::DomainResult;
use crate::domain::ids::{OrganizationId, PipelineId, ProjectId};
use crate::domain::pipeline::Pipeline;
use async_trait::async_trait;

#[async_trait]
pub trait PipelineRepository: Send + Sync {
    async fn create(&self, pipeline: &Pipeline) -> DomainResult<Pipeline>;

    async fn find_by_id(&self, id: &PipelineId) -> DomainResult<Pipeline>;

    /// Writes only if the row still carries `pipeline.version()`, and returns the row with the
    /// bumped version. A stale value is `Stale`; a missing row is `NotFound`.
    async fn update(&self, pipeline: &Pipeline) -> DomainResult<Pipeline>;

    /// Same version rule as `update`.
    async fn delete(&self, pipeline: &Pipeline) -> DomainResult<()>;

    async fn list_all(
        &self,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Pipeline>>;

    async fn list_by_project(
        &self,
        project_id: &ProjectId,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Pipeline>>;

    async fn list_by_organization(
        &self,
        organization_id: &OrganizationId,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Pipeline>>;
}
