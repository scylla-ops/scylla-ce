use chrono::Utc;
use scylla_domain::JobEvent;
use scylla_proto::agent::v1::{AgentUp, agent_up};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::error::ExecutionError;

/// The frames of one job. Once the control plane withdraws the job it ignores them, so none goes out.
#[derive(Clone)]
pub struct StatusPublisher {
    up_tx: mpsc::Sender<AgentUp>,
    job_id: String,
    withdrawn: CancellationToken,
}

impl StatusPublisher {
    #[must_use]
    pub fn new(up_tx: mpsc::Sender<AgentUp>, job_id: String, withdrawn: CancellationToken) -> Self {
        Self {
            up_tx,
            job_id,
            withdrawn,
        }
    }

    #[must_use]
    pub fn job_id(&self) -> &str {
        &self.job_id
    }

    /// Stamps the event with the time it happened, not the time the stream carries it.
    pub async fn emit(&self, event: JobEvent) -> Result<(), ExecutionError> {
        let status = scylla_proto::convert::job_event_to_status(&self.job_id, event, Utc::now());
        self.send(agent_up::Payload::Status(status)).await
    }

    pub async fn send(&self, payload: agent_up::Payload) -> Result<(), ExecutionError> {
        if self.withdrawn.is_cancelled() {
            return Ok(());
        }
        self.up_tx
            .send(AgentUp {
                payload: Some(payload),
            })
            .await
            .map_err(|e| ExecutionError::Publish(e.to_string()))
    }
}
