//! The messages of the agent about its jobs: the events and the batches of log lines.

use scylla_proto::agents::v1::open_stream_request::Message as Up;
use scylla_proto::agents::v1::{EventKind, JobEvent, LogBatch, OpenStreamRequest};
use scylla_proto::common::v1::Id;
use scylla_proto::pipelines::v1::LogLine;
use tokio::sync::mpsc;

/// The messages that wait for the stream. They wait while the agent connects again.
pub(crate) type Outbox = mpsc::Sender<OpenStreamRequest>;

/// The facts of one event.
#[derive(Default)]
pub(crate) struct Event<'a> {
    /// The node of a node event.
    pub(crate) node: &'a str,
    /// The new attempt of a retry.
    pub(crate) attempt: u32,
    /// The exit code of a node that ended.
    pub(crate) exit_code: Option<i32>,
    /// The error of a failure.
    pub(crate) error: String,
}

/// Sends an event of a job.
pub(crate) async fn send(outbox: &Outbox, job: &str, kind: EventKind, event: Event<'_>) {
    let message = JobEvent {
        job_id: Some(Id {
            value: job.to_owned(),
        }),
        kind: kind.into(),
        node_id: event.node.to_owned(),
        attempt: event.attempt,
        exit_code: event.exit_code,
        error: event.error.chars().take(4096).collect(),
    };
    let _ = outbox
        .send(OpenStreamRequest {
            message: Some(Up::Event(message)),
        })
        .await;
}

/// Sends a batch of lines of a node.
pub(crate) async fn logs(outbox: &Outbox, job: &str, node: &str, lines: Vec<LogLine>) {
    let batch = LogBatch {
        job_id: Some(Id {
            value: job.to_owned(),
        }),
        node_id: node.to_owned(),
        lines,
    };
    let _ = outbox
        .send(OpenStreamRequest {
            message: Some(Up::Logs(batch)),
        })
        .await;
}
