//! An open item and what it is about: a job, a fire of a trigger or an agent.

use time::OffsetDateTime;

use super::AttentionKind;
use crate::identity::apps::AppId;
use crate::projects::jobs::{JobId, JobStatus, NodeRun, NodeStatus};
use crate::projects::pipelines::{Node, NodeId, PipelineId};
use crate::projects::triggers::{FireId, FireSource, TriggerId};
use crate::projects::ProjectId;
use crate::Id;

/// The id of an open item: the bits of the id of its job, of its fire or of its agent.
pub type AttentionItemId = Id<AttentionItem>;

/// An open item that needs a person.
#[derive(Clone, Debug, PartialEq)]
pub struct AttentionItem {
    /// The kind.
    pub kind: AttentionKind,
    /// The time of the item: the end of a job, the creation of a job that waits, the due time of
    /// a fire, or the last seen time of an agent.
    pub at: OffsetDateTime,
    /// The open items of the same subject and kind that this item stands for, with it.
    pub count: u32,
    /// `true` when the item is after the last visit. The page sets it.
    pub is_new: bool,
    /// The project and the pipeline, or `None` for an agent.
    pub place: Option<Place>,
    /// What the item is about.
    pub subject: Subject,
}

impl AttentionItem {
    /// Returns the id of the item.
    pub fn id(&self) -> AttentionItemId {
        match &self.subject {
            Subject::Job(job) => job.id.retype(),
            Subject::Fire(fire) => fire.id.retype(),
            Subject::Agent(agent) => agent.id.retype(),
        }
    }
}

/// The project and the pipeline of an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    /// The project.
    pub project_id: ProjectId,
    /// The display name of the project.
    pub project_name: String,
    /// The pipeline.
    pub pipeline_id: PipelineId,
    /// The display name of the pipeline.
    pub pipeline_name: String,
}

/// What an item is about.
#[derive(Clone, Debug, PartialEq)]
pub enum Subject {
    /// A job that failed, that lost its agent, or that waits for an approval.
    Job(ItemJob),
    /// A fire that failed, or a cron time that the server missed.
    Fire(ItemFire),
    /// An agent that is not connected.
    Agent(ItemAgent),
}

/// The job of an item.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemJob {
    /// The id of the job.
    pub id: JobId,
    /// The state of the job.
    pub status: JobStatus,
    /// The error of the job.
    pub error: Option<String>,
    /// The branch of the commit of the job.
    pub branch: Option<String>,
    /// The agent of the job and its display name.
    pub agent: Option<(AppId, String)>,
    /// The node that failed, or that ran when the agent went away. The server sets it for the
    /// jobs of one page.
    pub node: Option<ItemNode>,
}

/// The node of the job of an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemNode {
    /// The id of the node.
    pub id: NodeId,
    /// The error of the node.
    pub error: Option<String>,
    /// The attempt, from 1.
    pub attempt: u32,
    /// The most attempts: 1 and the retries of the node.
    pub max_attempts: u32,
    /// The longest run of one attempt, in seconds.
    pub timeout_seconds: u32,
}

impl ItemNode {
    /// Finds the node of a job: the first node that failed, else for an orphaned job the first
    /// node that ran when the agent went away. `None` when no node fits.
    pub fn of(status: JobStatus, nodes: &[Node], runs: &[NodeRun]) -> Option<Self> {
        let failed = runs.iter().find(|run| run.status == NodeStatus::Failed);
        let stopped = || {
            runs.iter()
                .find(|run| run.status == NodeStatus::Cancelled && run.started_at.is_some())
        };
        let run = match status {
            JobStatus::Orphaned => failed.or_else(stopped),
            _ => failed,
        }?;
        let node = nodes.iter().find(|node| node.id == run.node);
        Some(Self {
            id: run.node.clone(),
            error: run.error.clone(),
            attempt: run.attempt,
            max_attempts: node.map_or(1, |node| node.retries.saturating_add(1)),
            timeout_seconds: node.map_or(Node::DEFAULT_TIMEOUT, |node| node.timeout_seconds),
        })
    }
}

/// The fire of an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemFire {
    /// The id of the fire.
    pub id: FireId,
    /// The trigger.
    pub trigger_id: TriggerId,
    /// The display name of the trigger.
    pub trigger_name: String,
    /// The cron schedule of the trigger, or `None` for another kind.
    pub schedule: Option<String>,
    /// What started the fire.
    pub source: FireSource,
    /// The reason of a fire that failed.
    pub error: Option<String>,
}

/// The agent of an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemAgent {
    /// The id of the agent.
    pub id: AppId,
    /// The display name of the agent.
    pub name: String,
    /// The last time when the server saw the agent.
    pub last_seen_at: OffsetDateTime,
}
