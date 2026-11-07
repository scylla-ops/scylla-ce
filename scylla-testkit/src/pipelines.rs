//! The `TestApp` helpers of `PipelineService` and of `JobService`.

use std::time::Duration;

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::pipelines::v1::job_service_client::JobServiceClient;
use scylla_proto::pipelines::v1::pipeline_service_client::PipelineServiceClient;
use scylla_proto::pipelines::v1::{
    CancelJobRequest, CreatePipelineRequest, DeleteJobRequest, DeletePipelineRequest,
    GetJobRequest, GetPipelineRequest, Job, ListPipelineJobsRequest, ListPipelineJobsResponse,
    ListProjectPipelinesRequest, ListProjectPipelinesResponse, ListWorkspaceJobsRequest,
    ListWorkspaceJobsResponse, ListWorkspacePipelinesRequest, ListWorkspacePipelinesResponse, Node,
    NodeList, Pipeline, RerunJobRequest, RunPipelineRequest, TailJobLogsRequest,
    TailJobLogsResponse, UpdatePipelineRequest,
};
use tonic::transport::Channel;
use tonic::Status;

use crate::rpc::page;
use crate::{id, TestApp};

impl TestApp {
    /// Returns a client of `PipelineService`.
    pub(crate) fn pipelines(&self) -> PipelineServiceClient<Channel> {
        PipelineServiceClient::new(self.channel.clone())
    }

    /// Returns a client of `JobService`.
    fn jobs(&self) -> JobServiceClient<Channel> {
        JobServiceClient::new(self.channel.clone())
    }

    /// Calls `CreatePipeline` as `actor` in a project.
    pub async fn create_pipeline(
        &self,
        actor: ActorId,
        project: &str,
        nodes: Vec<Node>,
    ) -> Result<Pipeline, Status> {
        let request = CreatePipelineRequest {
            project_id: Some(id(project)),
            name: "build".to_owned(),
            nodes,
            layout: None,
        };
        let request = self.authed(actor, request).await;
        let response = self
            .pipelines()
            .create_pipeline(request)
            .await?
            .into_inner();
        Ok(response.pipeline.expect("pipeline in response"))
    }

    /// Calls `GetPipeline` as `actor`.
    pub async fn get_pipeline(&self, actor: ActorId, pipeline: &str) -> Result<Pipeline, Status> {
        let request = GetPipelineRequest {
            pipeline_id: Some(id(pipeline)),
        };
        let request = self.authed(actor, request).await;
        let response = self.pipelines().get_pipeline(request).await?.into_inner();
        Ok(response.pipeline.expect("pipeline in response"))
    }

    /// Calls `UpdatePipeline` as `actor` with a version, a new name and new nodes.
    pub async fn update_pipeline(
        &self,
        actor: ActorId,
        pipeline: &str,
        version: i64,
        name: Option<&str>,
        nodes: Option<Vec<Node>>,
    ) -> Result<Pipeline, Status> {
        let request = UpdatePipelineRequest {
            pipeline_id: Some(id(pipeline)),
            version,
            name: name.map(str::to_owned),
            nodes: nodes.map(|nodes| NodeList { nodes }),
            layout: None,
        };
        let request = self.authed(actor, request).await;
        let response = self
            .pipelines()
            .update_pipeline(request)
            .await?
            .into_inner();
        Ok(response.pipeline.expect("pipeline in response"))
    }

    /// Calls `DeletePipeline` as `actor`.
    pub async fn delete_pipeline(&self, actor: ActorId, pipeline: &str) -> Result<(), Status> {
        let request = DeletePipelineRequest {
            pipeline_id: Some(id(pipeline)),
        };
        let request = self.authed(actor, request).await;
        self.pipelines().delete_pipeline(request).await?;
        Ok(())
    }

    /// Calls `ListProjectPipelines` as `actor` for one page.
    pub async fn list_project_pipelines(
        &self,
        actor: ActorId,
        project: &str,
        size: i32,
        token: &str,
    ) -> Result<ListProjectPipelinesResponse, Status> {
        let request = ListProjectPipelinesRequest {
            project_id: Some(id(project)),
            page: Some(page(size, token)),
        };
        let request = self.authed(actor, request).await;
        Ok(self
            .pipelines()
            .list_project_pipelines(request)
            .await?
            .into_inner())
    }

    /// Calls `ListWorkspacePipelines` as `actor` for the first page.
    pub async fn list_workspace_pipelines(
        &self,
        actor: ActorId,
        workspace: WorkspaceId,
    ) -> Result<ListWorkspacePipelinesResponse, Status> {
        let request = ListWorkspacePipelinesRequest {
            workspace_id: Some(id(workspace.to_string())),
            page: Some(page(0, "")),
        };
        let request = self.authed(actor, request).await;
        Ok(self
            .pipelines()
            .list_workspace_pipelines(request)
            .await?
            .into_inner())
    }

    /// Calls `RunPipeline` as `actor`, and returns the new job.
    pub async fn run_pipeline(&self, actor: ActorId, pipeline: &str) -> Result<Job, Status> {
        let request = RunPipelineRequest {
            pipeline_id: Some(id(pipeline)),
        };
        let request = self.authed(actor, request).await;
        let response = self.pipelines().run_pipeline(request).await?.into_inner();
        Ok(response.job.expect("job in response"))
    }

    /// Calls `GetJob` as `actor`.
    pub async fn get_job(&self, actor: ActorId, job: &str) -> Result<Job, Status> {
        let request = GetJobRequest {
            job_id: Some(id(job)),
        };
        let request = self.authed(actor, request).await;
        let response = self.jobs().get_job(request).await?.into_inner();
        Ok(response.job.expect("job in response"))
    }

    /// Calls `CancelJob` as `actor`.
    pub async fn cancel_job(&self, actor: ActorId, job: &str) -> Result<Job, Status> {
        let request = CancelJobRequest {
            job_id: Some(id(job)),
        };
        let request = self.authed(actor, request).await;
        let response = self.jobs().cancel_job(request).await?.into_inner();
        Ok(response.job.expect("job in response"))
    }

    /// Calls `DeleteJob` as `actor`.
    pub async fn delete_job(&self, actor: ActorId, job: &str) -> Result<(), Status> {
        let request = DeleteJobRequest {
            job_id: Some(id(job)),
        };
        let request = self.authed(actor, request).await;
        self.jobs().delete_job(request).await?;
        Ok(())
    }

    /// Calls `RerunJob` as `actor`, and returns the new job.
    pub async fn rerun_job(&self, actor: ActorId, job: &str) -> Result<Job, Status> {
        let request = RerunJobRequest {
            job_id: Some(id(job)),
        };
        let request = self.authed(actor, request).await;
        let response = self.jobs().rerun_job(request).await?.into_inner();
        Ok(response.job.expect("job in response"))
    }

    /// Calls `ListPipelineJobs` as `actor` for one page.
    pub async fn list_pipeline_jobs(
        &self,
        actor: ActorId,
        pipeline: &str,
        size: i32,
        token: &str,
    ) -> Result<ListPipelineJobsResponse, Status> {
        let request = ListPipelineJobsRequest {
            pipeline_id: Some(id(pipeline)),
            page: Some(page(size, token)),
        };
        let request = self.authed(actor, request).await;
        Ok(self.jobs().list_pipeline_jobs(request).await?.into_inner())
    }

    /// Calls `ListWorkspaceJobs` as `actor` for the first page, of one agent if it is set.
    pub async fn list_workspace_jobs(
        &self,
        actor: ActorId,
        workspace: WorkspaceId,
        agent: Option<&str>,
    ) -> Result<ListWorkspaceJobsResponse, Status> {
        let request = ListWorkspaceJobsRequest {
            workspace_id: Some(id(workspace.to_string())),
            agent_id: agent.map(id),
            page: Some(page(0, "")),
        };
        let request = self.authed(actor, request).await;
        Ok(self.jobs().list_workspace_jobs(request).await?.into_inner())
    }

    /// Calls `TailJobLogs` as `actor` and reads each batch until the end of the stream, for at
    /// most 10 seconds.
    pub async fn tail_job_logs(
        &self,
        actor: ActorId,
        job: &str,
        node_id: &str,
        after: &str,
    ) -> Result<Vec<TailJobLogsResponse>, Status> {
        let request = TailJobLogsRequest {
            job_id: Some(id(job)),
            node_id: node_id.to_owned(),
            after: after.to_owned(),
        };
        let request = self.authed(actor, request).await;
        let mut stream = self.jobs().tail_job_logs(request).await?.into_inner();
        let mut batches = Vec::new();
        let read = async {
            while let Some(batch) = stream.message().await? {
                batches.push(batch);
            }
            Ok::<_, Status>(())
        };
        tokio::time::timeout(Duration::from_secs(10), read)
            .await
            .map_err(|_| Status::deadline_exceeded("the tail did not end"))??;
        Ok(batches)
    }
}
