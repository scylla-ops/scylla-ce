//! The `TestApp` helpers of the GitHub check runs and their re-run: the report mode of a link, a
//! wait for a state that a report sent, and an agent that runs a job to a failed node with its log
//! lines.

use std::time::Duration;

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::pipelines::PipelineHolder;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::agents::v1::open_stream_request::Message as Up;
use scylla_proto::agents::v1::open_stream_response::Message as Down;
use scylla_proto::agents::v1::{
    EventKind, Hello, JobEvent, LogBatch, OpenStreamRequest, OpenStreamResponse,
};
use scylla_proto::apps::v1::Host;
use scylla_proto::integrations::v1::VcsReport;
use scylla_proto::pipelines::v1::{LogLine, LogStream};
use tokio::sync::mpsc::Sender;
use tokio::time::timeout;
use tonic::Streaming;

use crate::{id, text, TestApp};

/// The longest wait for a message of the server.
const WAIT: Duration = Duration::from_secs(10);

/// The longest wait for the sender of the reports, longer than its passes of 2 seconds.
const SENT_WAIT: Duration = Duration::from_secs(15);

/// The time between two reads of a wait.
const POLL: Duration = Duration::from_millis(100);

/// An agent with no feature, on its open stream.
pub struct RunAgent {
    /// The messages of the agent.
    up: Sender<OpenStreamRequest>,
    /// The messages of the server.
    down: Streaming<OpenStreamResponse>,
}

impl RunAgent {
    /// Takes the next dispatch, then runs its job: each node of `passed` completes, `failed`
    /// writes `lines` and fails, and the job fails. Returns the id of the job.
    pub async fn fail_next_job(
        &mut self,
        passed: &[&str],
        failed: &str,
        lines: &[String],
    ) -> String {
        // Take the dispatch.
        let job = loop {
            let message = timeout(WAIT, self.down.message()).await;
            let message = message.expect("a dispatch in 10 seconds");
            let message = message.expect("an open stream").expect("a message");
            if let Some(Down::Dispatch(dispatch)) = message.message {
                break text(&dispatch.job_id).to_owned();
            }
        };

        // Run the nodes that pass, then the node that fails with its lines.
        self.event(&job, EventKind::JobStarted, "").await;
        for node in passed {
            self.event(&job, EventKind::NodeStarted, node).await;
            self.event(&job, EventKind::NodeCompleted, node).await;
        }
        self.event(&job, EventKind::NodeStarted, failed).await;
        let lines = lines
            .iter()
            .map(|line| LogLine {
                stream: LogStream::Stdout.into(),
                text: line.clone(),
                at: None,
            })
            .collect();
        let batch = LogBatch {
            job_id: Some(id(&job)),
            node_id: failed.to_owned(),
            lines,
        };
        self.send(Up::Logs(batch)).await;
        self.event(&job, EventKind::NodeFailed, failed).await;
        self.event(&job, EventKind::JobFailed, "").await;
        job
    }

    /// Sends an event of a job, for a node if it is not empty.
    async fn event(&self, job: &str, kind: EventKind, node: &str) {
        let failed = matches!(kind, EventKind::NodeFailed | EventKind::JobFailed);
        let event = JobEvent {
            job_id: Some(id(job)),
            kind: kind.into(),
            node_id: node.to_owned(),
            attempt: 0,
            exit_code: Some(i32::from(failed)),
            error: if failed { "exit code 1" } else { "" }.to_owned(),
        };
        self.send(Up::Event(event)).await;
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
    /// Sets how the link of a project reports: `check_run` or `commit_status`.
    pub async fn set_link_report_mode(&self, project: &str, mode: &str) {
        sqlx::query("UPDATE repository_links SET report_mode = $2 WHERE project_id = $1::uuid")
            .bind(project)
            .bind(mode)
            .execute(&self.pool)
            .await
            .expect("set the report mode of a link");
    }

    /// Waits until the provider got a state of the first report of a job, such as `failure`, and
    /// returns the report. Fails after 15 seconds.
    pub async fn wait_for_sent_state(&self, actor: ActorId, job: &str, state: &str) -> VcsReport {
        let start = tokio::time::Instant::now();
        loop {
            let reports = self.job_reports(actor, job).await.reports;
            if let Some(report) = reports
                .into_iter()
                .find(|report| report.sent_state == state)
            {
                return report;
            }
            assert!(
                start.elapsed() < SENT_WAIT,
                "no report of {job} sent {state}"
            );
            tokio::time::sleep(POLL).await;
        }
    }

    /// Registers an agent with the role `runner` in a workspace and no feature, opens its
    /// stream, and waits for the welcome.
    pub async fn run_agent(&self, owner: ActorId, workspace: WorkspaceId) -> RunAgent {
        // Register the agent with the role, and open its stream.
        let (key, token) = self.app_in(owner, workspace, true).await;
        self.grant(&[PipelineHolder::RUNNER.of(workspace, key.actor())])
            .await;
        let (up, mut down) = self.open_stream(&token).await.expect("open the stream");

        // Say hello, and wait for the welcome.
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
        let welcome = timeout(WAIT, down.message()).await.expect("a welcome");
        let message = welcome.expect("an open stream").expect("a message").message;
        assert!(matches!(message, Some(Down::Welcome(_))), "{message:?}");
        RunAgent { up, down }
    }
}
