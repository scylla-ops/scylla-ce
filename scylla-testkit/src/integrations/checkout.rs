//! The `TestApp` helpers of the checkout of a job on an agent: an agent with its features, its
//! dispatches, its events and its log lines, a pipeline with checkout settings, and the rows of
//! the GitLab deploy tokens.

use std::time::Duration;

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::pipelines::PipelineHolder;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::agents::v1::open_stream_request::Message as Up;
use scylla_proto::agents::v1::open_stream_response::Message as Down;
use scylla_proto::agents::v1::{
    Dispatch, EventKind, Hello, JobEvent, LogBatch, OpenStreamRequest, OpenStreamResponse,
};
use scylla_proto::apps::v1::Host;
use scylla_proto::pipelines::v1::{
    CheckoutSettings, CreatePipelineRequest, LogLine, LogStream, Node, Pipeline,
    UpdatePipelineRequest,
};
use tokio::sync::mpsc::Sender;
use tokio::time::timeout;
use tonic::{Status, Streaming};

use crate::{id, TestApp, TestKey};

/// An agent of a test with its open stream.
pub struct TestAgent {
    /// The machine of the agent.
    pub key: TestKey,
    /// The messages of the agent.
    up: Sender<OpenStreamRequest>,
    /// The messages of the server.
    down: Streaming<OpenStreamResponse>,
}

impl TestAgent {
    /// Returns the next dispatch, or `None` if none comes in `wait`. Other messages are skipped.
    pub async fn dispatch_in(&mut self, wait: Duration) -> Option<Dispatch> {
        let read = async {
            while let Some(message) = self.down.message().await.ok()? {
                if let Some(Down::Dispatch(dispatch)) = message.message {
                    return Some(dispatch);
                }
            }
            None
        };
        timeout(wait, read).await.ok().flatten()
    }

    /// Sends an event of a job, with an error for a failure.
    pub async fn event(&self, job: &str, kind: EventKind, error: &str) {
        let event = JobEvent {
            job_id: Some(id(job)),
            kind: kind.into(),
            node_id: String::new(),
            attempt: 0,
            exit_code: None,
            error: error.to_owned(),
        };
        self.send(Up::Event(event)).await;
    }

    /// Sends one log line of a node of a job, or of its checkout with the node `.checkout`.
    pub async fn log(&self, job: &str, node: &str, text: &str) {
        let batch = LogBatch {
            job_id: Some(id(job)),
            node_id: node.to_owned(),
            lines: vec![LogLine {
                stream: LogStream::Stdout.into(),
                text: text.to_owned(),
                at: None,
            }],
        };
        self.send(Up::Logs(batch)).await;
    }

    /// Sends a message of the agent.
    async fn send(&self, message: Up) {
        let message = OpenStreamRequest {
            message: Some(message),
        };
        self.up.send(message).await.expect("an open stream");
    }
}

impl TestApp {
    /// Registers an agent with the role `runner` in a workspace, opens its stream with these
    /// features, and waits for the welcome.
    pub async fn agent_with(
        &self,
        owner: ActorId,
        workspace: WorkspaceId,
        features: &[&str],
    ) -> TestAgent {
        // Register the agent with the role, and open its stream.
        let (key, token) = self.app_in(owner, workspace, true).await;
        self.grant(&[PipelineHolder::RUNNER.of(workspace, key.actor())])
            .await;
        let (up, mut down) = self.open_stream(&token).await.expect("open the stream");

        // Say hello with the features, and wait for the welcome.
        let host = Host {
            version: "1.0.0".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            hostname: "runner".to_owned(),
            cpu_count: 4,
            memory_bytes: 1 << 30,
            capacity: 1,
            container_runtime: String::new(),
            features: features.iter().map(|name| (*name).to_owned()).collect(),
        };
        let hello = Up::Hello(Hello {
            host: Some(host),
            running_jobs: Vec::new(),
        });
        let message = OpenStreamRequest {
            message: Some(hello),
        };
        up.send(message).await.expect("send the hello");
        let welcome = timeout(Duration::from_secs(10), down.message()).await;
        let welcome = welcome.expect("a welcome in 10 seconds");
        let message = welcome.expect("an open stream").expect("a message").message;
        assert!(matches!(message, Some(Down::Welcome(_))), "{message:?}");
        TestAgent { key, up, down }
    }

    /// Calls `CreatePipeline` as `actor` in a project, with checkout settings.
    pub async fn create_pipeline_with_checkout(
        &self,
        actor: ActorId,
        project: &str,
        nodes: Vec<Node>,
        checkout: CheckoutSettings,
    ) -> Result<Pipeline, Status> {
        let request = CreatePipelineRequest {
            project_id: Some(id(project)),
            name: "build".to_owned(),
            nodes,
            layout: None,
            checkout: Some(checkout),
        };
        let request = self.authed(actor, request).await;
        let response = self.pipelines().create_pipeline(request).await?;
        Ok(response.into_inner().pipeline.expect("pipeline"))
    }

    /// Calls `UpdatePipeline` as `actor` with new checkout settings only.
    pub async fn update_pipeline_checkout(
        &self,
        actor: ActorId,
        pipeline: &str,
        version: i64,
        checkout: CheckoutSettings,
    ) -> Result<Pipeline, Status> {
        let request = UpdatePipelineRequest {
            pipeline_id: Some(id(pipeline)),
            version,
            name: None,
            nodes: None,
            layout: None,
            checkout: Some(checkout),
        };
        let request = self.authed(actor, request).await;
        let response = self.pipelines().update_pipeline(request).await?;
        Ok(response.into_inner().pipeline.expect("pipeline"))
    }

    /// Returns the rows of the GitLab deploy tokens: the job, as the text of its id, the id of
    /// the token on GitLab, and the life of the token in whole seconds from its row.
    pub async fn checkout_tokens(&self) -> Vec<(Option<String>, String, i64)> {
        let rows: Vec<(Option<uuid::Uuid>, String, i64)> = sqlx::query_as(
            "SELECT job_id, token_external_id,
                    round(extract(epoch FROM expires_at - created_at))::bigint
             FROM vcs_checkout_tokens",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read the deploy tokens");
        rows.into_iter()
            .map(|(job, token, life)| (job.map(|job| job.to_string()), token, life))
            .collect()
    }
}
