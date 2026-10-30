use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum JobEvent {
    JobStarted,
    NodeStarted { node_id: String },
    NodeCompleted { node_id: String },
    NodeFailed { node_id: String, error: String },
    NodeSkipped { node_id: String },
    NodeRetrying { node_id: String, attempt: u32 },
    JobCompleted,
    JobFailed { error: String },
}
