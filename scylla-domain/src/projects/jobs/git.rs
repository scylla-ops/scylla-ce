//! The Git parts of a job: the approval of a job from a fork, the pipeline file of a job of a
//! file pipeline, and the checkout of its code.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use super::NodeStatus;
use crate::identity::actors::ActorId;
use crate::integrations::sources::ApprovalState;

/// The approval of a job from a fork.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobApproval {
    /// The state.
    pub state: ApprovalState,
    /// The actor that approved or rejected the job, or `None` while the job waits.
    pub decided_by: Option<ActorId>,
    /// The time of the decision, or `None` while the job waits.
    pub decided_at: Option<OffsetDateTime>,
}

impl JobApproval {
    /// Makes the approval of a job that waits for an editor.
    pub fn waiting() -> Self {
        Self {
            state: ApprovalState::Waiting,
            decided_by: None,
            decided_at: None,
        }
    }
}

/// A pipeline file in the repository, at the commit of a job.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineFileRef {
    /// The path, for example `.scylla/pipelines/build.toml`.
    pub path: String,
    /// The SHA of the blob of the file.
    pub blob_sha: String,
    /// The commit.
    pub commit_sha: String,
    /// The page of the file at the commit on the provider.
    pub url: String,
}

/// The checkout of the code of a job, before its first node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckoutRun {
    /// The state.
    pub status: NodeStatus,
    /// The start.
    pub started_at: Option<OffsetDateTime>,
    /// The end.
    pub finished_at: Option<OffsetDateTime>,
    /// The error of a failed checkout.
    pub error: Option<String>,
}
