use crate::application::agent::AgentStream;
use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::domain::errors::DomainResult;
use crate::domain::ids::{AppId, JobId, OrganizationId, PipelineId, ProjectId};
use crate::domain::job::Job;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scylla_auth::authz::Visibility;

#[derive(Debug, Clone, Copy)]
pub enum JobScope<'a> {
    All,
    Pipeline(&'a PipelineId),
    Project(&'a ProjectId),
    Organization(&'a OrganizationId),
}

/// `update` and `delete` are versioned: a job that changed since it was read gives `Stale`.
/// The placement is the store's alone: `claim_next` puts a job on one stream of an agent, and
/// `release` returns the jobs of one stream to the pool. Both move the version.
#[async_trait]
pub trait JobRepository: Send + Sync {
    async fn create(&self, job: &Job) -> DomainResult<Job>;

    async fn find_by_id(&self, id: &JobId) -> DomainResult<Job>;

    async fn update(&self, job: &Job) -> DomainResult<Job>;

    async fn delete(&self, job: &Job) -> DomainResult<()>;

    /// Places the oldest pending job that `visible` covers on the stream, only while its agent
    /// has no job that has not ended. The project is the one of the job's pipeline.
    async fn claim_next(
        &self,
        stream: &AgentStream,
        visible: &Visibility,
    ) -> DomainResult<Option<(Job, ProjectId)>>;

    /// Returns to the pool the jobs placed on this stream that have not started.
    async fn release(&self, stream: &AgentStream) -> DomainResult<u64>;

    /// The streams that hold a job that has not started.
    async fn pending_streams(&self) -> DomainResult<Vec<AgentStream>>;

    /// The jobs that have not ended, for each agent that has one.
    async fn active_jobs(&self, agents: &[AppId]) -> DomainResult<Vec<(AppId, u32)>>;

    async fn list_live(&self, scope: JobScope<'_>) -> DomainResult<Vec<Job>>;

    async fn list_running_on(&self, agent: &AppId) -> DomainResult<Vec<Job>>;

    /// The running jobs whose agent lost its row or was last seen before `seen_before`.
    async fn list_stranded(&self, seen_before: DateTime<Utc>) -> DomainResult<Vec<Job>>;

    async fn list_all(
        &self,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Job>>;

    async fn list_by_pipeline(
        &self,
        pipeline_id: &PipelineId,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Job>>;

    async fn list_by_project(
        &self,
        project_id: &ProjectId,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Job>>;

    async fn list_by_organization(
        &self,
        organization_id: &OrganizationId,
        agent: Option<&AppId>,
        pagination: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Job>>;
}
