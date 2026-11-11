//! The `TestApp` helpers of the commit context of the runs: a run on a branch, a tag or a SHA, the
//! job lists with filters, a change of the source of a job, and the first dispatch to an agent.

use std::time::Duration;

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::pipelines::PipelineHolder;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::agents::v1::open_stream_request::Message as Up;
use scylla_proto::agents::v1::open_stream_response::Message as Down;
use scylla_proto::agents::v1::{Dispatch, Hello, OpenStreamRequest};
use scylla_proto::apps::v1::Host;
use scylla_proto::pipelines::v1::job_service_client::JobServiceClient;
use scylla_proto::pipelines::v1::{
    git_ref, GitRef, Job, JobFilter, ListPipelineJobsRequest, ListWorkspaceJobsRequest,
    RunPipelineRequest,
};
use serde_json::json;
use tokio::sync::mpsc::Sender;
use tonic::Status;

use crate::{id, TestApp};

impl TestApp {
    /// Calls `RunPipeline` as `actor` on a branch, a tag or a SHA of the linked repository.
    pub async fn run_pipeline_on(
        &self,
        actor: ActorId,
        pipeline: &str,
        kind: git_ref::Kind,
    ) -> Result<Job, Status> {
        let request = RunPipelineRequest {
            pipeline_id: Some(id(pipeline)),
            r#ref: Some(GitRef { kind: Some(kind) }),
        };
        let request = self.authed(actor, request).await;
        let response = self.pipelines().run_pipeline(request).await?.into_inner();
        Ok(response.job.expect("job in response"))
    }

    /// Calls `ListPipelineJobs` as `actor` with a filter, and returns the jobs of the first page.
    pub async fn list_pipeline_jobs_with(
        &self,
        actor: ActorId,
        pipeline: &str,
        filter: JobFilter,
    ) -> Result<Vec<Job>, Status> {
        let request = ListPipelineJobsRequest {
            pipeline_id: Some(id(pipeline)),
            page: None,
            filter: Some(filter),
        };
        let request = self.authed(actor, request).await;
        let mut client = JobServiceClient::new(self.channel.clone());
        Ok(client.list_pipeline_jobs(request).await?.into_inner().jobs)
    }

    /// Calls `ListWorkspaceJobs` as `actor` with a filter, and returns the jobs of the first page.
    pub async fn list_workspace_jobs_with(
        &self,
        actor: ActorId,
        workspace: WorkspaceId,
        filter: JobFilter,
    ) -> Result<Vec<Job>, Status> {
        let request = ListWorkspaceJobsRequest {
            workspace_id: Some(id(workspace.to_string())),
            agent_id: None,
            page: None,
            filter: Some(filter),
        };
        let request = self.authed(actor, request).await;
        let mut client = JobServiceClient::new(self.channel.clone());
        Ok(client.list_workspace_jobs(request).await?.into_inner().jobs)
    }

    /// Changes the source of a job: the login of the author, one pull request into `main` if
    /// `pull` is set, and the fork of the commit if `fork` is set.
    pub async fn set_job_source(
        &self,
        job: &str,
        author: &str,
        pull: Option<u32>,
        fork: Option<&str>,
    ) {
        let pulls: Vec<_> = pull
            .into_iter()
            .map(|number| {
                json!({
                    "number": number,
                    "title": format!("Change {number}"),
                    "url": format!("https://github.com/octo/app/pull/{number}"),
                    "source_branch": "feature",
                    "target_branch": "main",
                })
            })
            .collect();
        sqlx::query(
            "UPDATE job_sources SET author_login = $2, pull_requests = $3, from_fork = $4,
                                    fork_repository = $5, changes_pipeline_files = $4
             WHERE job_id = $1::uuid",
        )
        .bind(job)
        .bind(author)
        .bind(json!(pulls))
        .bind(fork.is_some())
        .bind(fork)
        .execute(&self.pool)
        .await
        .expect("change a job source");
    }

    /// Registers an agent with the role `runner` in a workspace, opens its stream, and returns
    /// the first dispatch that it gets in 10 seconds. The stream stays open while the sender
    /// lives.
    pub async fn first_dispatch(
        &self,
        owner: ActorId,
        workspace: WorkspaceId,
    ) -> (Dispatch, Sender<OpenStreamRequest>) {
        // Register the agent with the role, and open its stream.
        let (key, token) = self.app_in(owner, workspace, true).await;
        self.grant(&[PipelineHolder::RUNNER.of(workspace, key.actor())])
            .await;
        let (up, mut down) = self.open_stream(&token).await.expect("open the stream");
        let host = Host {
            version: "1.0.0".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            hostname: "runner".to_owned(),
            cpu_count: 4,
            memory_bytes: 1 << 30,
            capacity: 1,
            container_runtime: String::new(),
            features: Vec::new(),
        };
        let hello = Up::Hello(Hello {
            host: Some(host),
            running_jobs: Vec::new(),
        });
        let message = OpenStreamRequest {
            message: Some(hello),
        };
        up.send(message).await.expect("send the hello");

        // Read the messages until the dispatch.
        let read = async {
            while let Some(message) = down.message().await.expect("an open stream") {
                if let Some(Down::Dispatch(dispatch)) = message.message {
                    return dispatch;
                }
            }
            panic!("the stream ended with no dispatch");
        };
        let dispatch = tokio::time::timeout(Duration::from_secs(10), read)
            .await
            .expect("a dispatch in 10 seconds");
        (dispatch, up)
    }
}
