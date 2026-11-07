//! The state machine of a job and of its nodes: the events of the agent, and the end of a job.

use time::OffsetDateTime;

use super::{error_text, Job, JobStatus, NodeStatus};
use crate::projects::pipelines::{Node, NodeId, PortName, Step};
use crate::DomainError;

/// The state of one node in a job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeRun {
    /// The node.
    pub node: NodeId,
    /// The state.
    pub status: NodeStatus,
    /// The current attempt, from 1. 0 before the start.
    pub attempt: u32,
    /// The start of the first attempt.
    pub started_at: Option<OffsetDateTime>,
    /// The end.
    pub finished_at: Option<OffsetDateTime>,
    /// The exit code of the last attempt.
    pub exit_code: Option<i32>,
    /// The error of a failed node.
    pub error: Option<String>,
    /// The ports that a completed switch opened. Empty for another node.
    pub ports: Vec<PortName>,
}

impl NodeRun {
    /// Makes the state of a node that waits.
    pub fn pending(node: &Node) -> Self {
        Self {
            node: node.id.clone(),
            status: NodeStatus::Pending,
            attempt: 0,
            started_at: None,
            finished_at: None,
            exit_code: None,
            error: None,
            ports: Vec::new(),
        }
    }
}

/// An event of the agent about a job that it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobEvent {
    /// The agent starts the job.
    JobStarted,
    /// A node starts its first attempt.
    NodeStarted(NodeId),
    /// A node starts a new attempt after a failure.
    NodeRetrying {
        /// The node.
        node: NodeId,
        /// The new attempt: one more than the current attempt.
        attempt: u32,
    },
    /// A node completed.
    NodeCompleted {
        /// The node.
        node: NodeId,
        /// The exit code.
        exit_code: Option<i32>,
        /// The ports that a switch opened. Empty for another node.
        ports: Vec<PortName>,
    },
    /// A node failed after its last attempt.
    NodeFailed {
        /// The node.
        node: NodeId,
        /// The exit code, if the process ended.
        exit_code: Option<i32>,
        /// The error.
        error: String,
    },
    /// A node does not run: a node failed, or a need of the node did not open.
    NodeSkipped(NodeId),
    /// Each node completed or was skipped.
    JobCompleted,
    /// The job failed.
    JobFailed(String),
}

impl Job {
    /// Applies an event of the agent at `now`. An event that does not fit the state gives
    /// `NotAllowed` and changes nothing.
    pub fn apply(&mut self, event: &JobEvent, now: OffsetDateTime) -> Result<(), DomainError> {
        match event {
            JobEvent::JobStarted if self.status == JobStatus::Pending => {
                self.status = JobStatus::Running;
                self.started_at = Some(now);
                Ok(())
            }
            JobEvent::JobFailed(error) if !self.status.is_final() => {
                self.end(JobStatus::Failed, Some(error), now);
                Ok(())
            }
            JobEvent::JobCompleted if self.status == JobStatus::Running => {
                let done = |status| matches!(status, NodeStatus::Completed | NodeStatus::Skipped);
                if !self.runs.iter().all(|run| done(run.status)) {
                    return Err(DomainError::NotAllowed);
                }
                self.end(JobStatus::Completed, None, now);
                Ok(())
            }
            JobEvent::NodeStarted(_)
            | JobEvent::NodeRetrying { .. }
            | JobEvent::NodeCompleted { .. }
            | JobEvent::NodeFailed { .. }
            | JobEvent::NodeSkipped(_)
                if self.status == JobStatus::Running =>
            {
                self.apply_node(event, now)
            }
            _ => Err(DomainError::NotAllowed),
        }
    }

    /// Applies an event of a node of a running job.
    fn apply_node(&mut self, event: &JobEvent, now: OffsetDateTime) -> Result<(), DomainError> {
        // Find the node and its limits.
        let node = match event {
            JobEvent::NodeStarted(node) | JobEvent::NodeSkipped(node) => node,
            JobEvent::NodeRetrying { node, .. }
            | JobEvent::NodeCompleted { node, .. }
            | JobEvent::NodeFailed { node, .. } => node,
            _ => return Err(DomainError::NotAllowed),
        };
        let definition = self
            .nodes
            .iter()
            .find(|candidate| &candidate.id == node)
            .ok_or(DomainError::NotAllowed)?;
        let retries = definition.retries;
        let run = self
            .runs
            .iter_mut()
            .find(|run| &run.node == node)
            .ok_or(DomainError::NotAllowed)?;

        // Change the state of the node, if the event fits it.
        match (event, run.status) {
            (JobEvent::NodeStarted(_), NodeStatus::Pending) => {
                run.status = NodeStatus::Running;
                run.attempt = 1;
                run.started_at = Some(now);
            }
            (JobEvent::NodeRetrying { attempt, .. }, NodeStatus::Running)
                if *attempt == run.attempt + 1 && *attempt <= retries + 1 =>
            {
                run.attempt = *attempt;
            }
            (
                JobEvent::NodeCompleted {
                    exit_code, ports, ..
                },
                NodeStatus::Running,
            ) => {
                let fits = match &definition.step {
                    Step::Switch(switch) => switch.allows(ports),
                    _ => ports.is_empty(),
                };
                if !fits {
                    return Err(DomainError::NotAllowed);
                }
                run.status = NodeStatus::Completed;
                run.exit_code = *exit_code;
                run.ports = ports.clone();
                run.finished_at = Some(now);
            }
            (
                JobEvent::NodeFailed {
                    exit_code, error, ..
                },
                NodeStatus::Running,
            ) => {
                run.status = NodeStatus::Failed;
                run.exit_code = *exit_code;
                run.error = Some(error_text(error));
                run.finished_at = Some(now);
            }
            (JobEvent::NodeSkipped(_), NodeStatus::Pending | NodeStatus::Running) => {
                run.status = NodeStatus::Skipped;
                run.finished_at = Some(now);
            }
            _ => return Err(DomainError::NotAllowed),
        }
        Ok(())
    }

    /// Ends a job that is not final: the nodes that did not end become `Cancelled`. A final
    /// job does not change. Returns `true` if the job changed.
    pub fn end(&mut self, status: JobStatus, error: Option<&str>, now: OffsetDateTime) -> bool {
        if self.status.is_final() || !status.is_final() {
            return false;
        }
        self.status = status;
        self.error = error.map(error_text);
        self.finished_at = Some(now);
        for run in self.runs.iter_mut().filter(|run| !run.status.is_final()) {
            run.status = NodeStatus::Cancelled;
            run.finished_at = Some(now);
        }
        true
    }
}
