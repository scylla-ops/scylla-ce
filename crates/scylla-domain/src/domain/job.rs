mod event;
mod log;
mod log_stream;
mod node_state;
mod origin;
mod status;

pub use event::*;
pub use log::*;
pub use log_stream::*;
pub use node_state::*;
pub use origin::*;
pub use status::*;

use crate::domain::clock;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{AppId, JobId, PipelineId};
use crate::domain::pipeline::{EnvKey, EnvValue, NodeId, Pipeline, PipelineNode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum NodeExecution {
    Pending,
    Running {
        started_at: DateTime<Utc>,
    },
    Finished {
        started_at: Option<DateTime<Utc>>,
        finished_at: DateTime<Utc>,
        outcome: NodeOutcome,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeOutcome {
    Completed,
    Failed,
    Cancelled,
    Skipped,
}

impl NodeOutcome {
    #[must_use]
    fn as_node_state(self) -> NodeState {
        match self {
            Self::Completed => NodeState::Completed,
            Self::Failed => NodeState::Failed,
            Self::Cancelled => NodeState::Cancelled,
            Self::Skipped => NodeState::Skipped,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobNode {
    node_id: NodeId,
    execution: NodeExecution,
}

impl JobNode {
    #[must_use]
    pub fn from_persistence(node_id: NodeId, execution: NodeExecution) -> Self {
        Self { node_id, execution }
    }

    #[must_use]
    pub fn new(node_id: NodeId) -> Self {
        Self {
            node_id,
            execution: NodeExecution::Pending,
        }
    }

    fn start(self, started_at: DateTime<Utc>) -> DomainResult<Self> {
        match self.execution {
            NodeExecution::Pending => Ok(Self {
                node_id: self.node_id,
                execution: NodeExecution::Running { started_at },
            }),
            NodeExecution::Running { .. } | NodeExecution::Finished { .. } => Err(
                DomainError::business_rule("Node must be pending to start execution"),
            ),
        }
    }

    fn finish(self, outcome: NodeOutcome, finished_at: DateTime<Utc>) -> DomainResult<Self> {
        match self.execution {
            NodeExecution::Running { started_at } => Ok(Self {
                node_id: self.node_id,
                execution: NodeExecution::Finished {
                    started_at: Some(started_at),
                    finished_at,
                    outcome,
                },
            }),
            NodeExecution::Pending => Err(DomainError::business_rule(
                "Node must be running to finish execution",
            )),
            NodeExecution::Finished { .. } => {
                Err(DomainError::business_rule("Node has already finished"))
            }
        }
    }

    fn skip(self, finished_at: DateTime<Utc>) -> DomainResult<Self> {
        let started_at = match self.execution {
            NodeExecution::Pending => None,
            NodeExecution::Running { started_at } => Some(started_at),
            NodeExecution::Finished { .. } => {
                return Err(DomainError::business_rule(
                    "Cannot skip a node already in terminal state",
                ));
            }
        };
        Ok(Self {
            node_id: self.node_id,
            execution: NodeExecution::Finished {
                started_at,
                finished_at,
                outcome: NodeOutcome::Skipped,
            },
        })
    }

    fn cancel_if_active(self, finished_at: DateTime<Utc>) -> Self {
        let started_at = match self.execution {
            NodeExecution::Pending => None,
            NodeExecution::Running { started_at } => Some(started_at),
            NodeExecution::Finished { .. } => return self,
        };
        Self {
            node_id: self.node_id,
            execution: NodeExecution::Finished {
                started_at,
                finished_at,
                outcome: NodeOutcome::Cancelled,
            },
        }
    }

    #[must_use]
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    #[must_use]
    pub fn execution(&self) -> &NodeExecution {
        &self.execution
    }

    #[must_use]
    pub fn state(&self) -> NodeState {
        match &self.execution {
            NodeExecution::Pending => NodeState::Pending,
            NodeExecution::Running { .. } => NodeState::Running,
            NodeExecution::Finished { outcome, .. } => outcome.as_node_state(),
        }
    }

    #[must_use]
    pub fn started_at(&self) -> Option<DateTime<Utc>> {
        match &self.execution {
            NodeExecution::Pending => None,
            NodeExecution::Running { started_at } => Some(*started_at),
            NodeExecution::Finished { started_at, .. } => *started_at,
        }
    }

    #[must_use]
    pub fn finished_at(&self) -> Option<DateTime<Utc>> {
        match &self.execution {
            NodeExecution::Pending | NodeExecution::Running { .. } => None,
            NodeExecution::Finished { finished_at, .. } => Some(*finished_at),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalOutcome {
    Completed,
    Failed,
    Cancelled,
    Orphaned,
}

impl TerminalOutcome {
    fn from_status(status: JobStatus) -> DomainResult<Self> {
        match status {
            JobStatus::Completed => Ok(Self::Completed),
            JobStatus::Failed => Ok(Self::Failed),
            JobStatus::Cancelled => Ok(Self::Cancelled),
            JobStatus::Orphaned => Ok(Self::Orphaned),
            JobStatus::Pending | JobStatus::Running => Err(DomainError::validation(
                "non-terminal status has no terminal outcome",
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub enum JobState {
    Pending,
    Running {
        started_at: DateTime<Utc>,
    },
    Terminal {
        outcome: TerminalOutcome,
        started_at: Option<DateTime<Utc>>,
        finished_at: DateTime<Utc>,
    },
}

impl JobState {
    pub fn from_columns(
        status: JobStatus,
        started_at: Option<DateTime<Utc>>,
        finished_at: Option<DateTime<Utc>>,
    ) -> DomainResult<Self> {
        match status {
            JobStatus::Pending => Ok(Self::Pending),
            JobStatus::Running => Ok(Self::Running {
                started_at: started_at.ok_or_else(|| {
                    DomainError::validation("running job is missing its started_at")
                })?,
            }),
            JobStatus::Completed
            | JobStatus::Failed
            | JobStatus::Cancelled
            | JobStatus::Orphaned => Ok(Self::Terminal {
                outcome: TerminalOutcome::from_status(status)?,
                started_at,
                finished_at: finished_at.ok_or_else(|| {
                    DomainError::validation("terminal job is missing its finished_at")
                })?,
            }),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Job {
    id: JobId,
    pipeline_id: PipelineId,
    state: JobState,
    /// Written by the store alone: it places the job, returns it to the pool, or loses the
    /// agent row (`ON DELETE SET NULL`). Not part of `JobState`.
    agent_app_id: Option<AppId>,
    /// The nodes of the pipeline when the job was created; the job runs these and no others.
    nodes: Vec<PipelineNode>,
    node_executions: Vec<JobNode>,
    /// Persisted so a retried dispatch is identical to the first.
    inputs: Vec<(EnvKey, EnvValue)>,
    origin: JobOrigin,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    /// The row version the value was read at. The store checks it on every write and bumps
    /// it itself; the domain never changes it.
    version: u64,
}

impl Job {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn from_persistence(
        id: JobId,
        pipeline_id: PipelineId,
        state: JobState,
        agent_app_id: Option<AppId>,
        nodes: Vec<PipelineNode>,
        node_executions: Vec<JobNode>,
        inputs: Vec<(EnvKey, EnvValue)>,
        origin: JobOrigin,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        version: u64,
    ) -> Self {
        Self {
            id,
            pipeline_id,
            state,
            agent_app_id,
            nodes,
            node_executions,
            inputs,
            origin,
            created_at,
            updated_at,
            version,
        }
    }

    #[must_use]
    pub fn create_from_pipeline(pipeline: &Pipeline, origin: JobOrigin) -> Self {
        let now = clock::now();
        let nodes = pipeline.nodes().to_vec();
        let node_executions = nodes
            .iter()
            .map(|node| JobNode::new(node.id().clone()))
            .collect();

        Self {
            id: JobId::generate(),
            pipeline_id: pipeline.id().clone(),
            state: JobState::Pending,
            agent_app_id: None,
            nodes,
            node_executions,
            inputs: Vec::new(),
            origin,
            created_at: now,
            updated_at: now,
            version: 0,
        }
    }

    #[must_use]
    pub fn with_inputs(mut self, inputs: Vec<(EnvKey, EnvValue)>) -> Self {
        self.inputs = inputs;
        self
    }

    pub fn start(mut self, at: DateTime<Utc>) -> DomainResult<Self> {
        match self.state {
            JobState::Pending => {
                self.state = JobState::Running { started_at: at };
                self.updated_at = at;
                Ok(self)
            }
            JobState::Running { .. } | JobState::Terminal { .. } => Err(
                DomainError::business_rule("only a pending job can start running"),
            ),
        }
    }

    pub fn complete(self, at: DateTime<Utc>) -> DomainResult<Self> {
        self.end(TerminalOutcome::Completed, at)
    }

    pub fn fail(self, at: DateTime<Utc>) -> DomainResult<Self> {
        self.end(TerminalOutcome::Failed, at)
    }

    pub fn cancel(self, at: DateTime<Utc>) -> DomainResult<Self> {
        self.end(TerminalOutcome::Cancelled, at)
    }

    pub fn orphan(self, at: DateTime<Utc>) -> DomainResult<Self> {
        self.end(TerminalOutcome::Orphaned, at)
    }

    /// Every way a job ends: a node that has not finished ends as cancelled.
    fn end(mut self, outcome: TerminalOutcome, at: DateTime<Utc>) -> DomainResult<Self> {
        let started_at = match self.state {
            JobState::Running { started_at } => Some(started_at),
            JobState::Pending if outcome != TerminalOutcome::Completed => None,
            JobState::Pending => {
                return Err(DomainError::business_rule(
                    "only a running job can complete",
                ));
            }
            JobState::Terminal { .. } => {
                return Err(DomainError::business_rule("the job has already ended"));
            }
        };
        self.state = JobState::Terminal {
            outcome,
            started_at,
            finished_at: at,
        };
        self.node_executions = std::mem::take(&mut self.node_executions)
            .into_iter()
            .map(|node| node.cancel_if_active(at))
            .collect();
        self.updated_at = at;
        Ok(self)
    }

    pub fn apply_node_started(mut self, node_id: &NodeId, at: DateTime<Utc>) -> DomainResult<Self> {
        self.transition_node(node_id, at, |node| node.start(at))?;
        Ok(self)
    }

    pub fn apply_node_finished(
        mut self,
        node_id: &NodeId,
        outcome: NodeOutcome,
        at: DateTime<Utc>,
    ) -> DomainResult<Self> {
        self.transition_node(node_id, at, |node| node.finish(outcome, at))?;
        Ok(self)
    }

    pub fn apply_node_skipped(mut self, node_id: &NodeId, at: DateTime<Utc>) -> DomainResult<Self> {
        self.transition_node(node_id, at, |node| node.skip(at))?;
        Ok(self)
    }

    fn transition_node(
        &mut self,
        node_id: &NodeId,
        at: DateTime<Utc>,
        transition: impl FnOnce(JobNode) -> DomainResult<JobNode>,
    ) -> DomainResult<()> {
        if !matches!(self.state, JobState::Running { .. }) {
            return Err(DomainError::business_rule(
                "a node event needs a running job",
            ));
        }
        let node = self
            .node_executions
            .iter_mut()
            .find(|e| e.node_id() == node_id)
            .ok_or_else(|| DomainError::validation(format!("Node not found: {node_id}")))?;
        *node = transition(node.clone())?;
        self.updated_at = at;
        Ok(())
    }

    /// A report counts only from the agent the job is placed on, and only until the job ends.
    pub fn ensure_live_on(&self, agent: &AppId) -> DomainResult<()> {
        if self.agent_app_id.as_ref() == Some(agent) && !self.is_terminal() {
            Ok(())
        } else {
            Err(DomainError::business_rule(
                "the job is not live on this agent",
            ))
        }
    }

    /// A pending node has no execution behind it: log rows keyed to it are stale or early.
    #[must_use]
    pub fn logs_readable_for(&self, node_id: &NodeId) -> bool {
        self.find_execution(node_id)
            .is_some_and(|n| n.state() != NodeState::Pending)
    }

    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self.state, JobState::Terminal { .. })
    }

    #[must_use]
    pub fn find_execution(&self, node_id: &NodeId) -> Option<&JobNode> {
        self.node_executions.iter().find(|e| e.node_id() == node_id)
    }

    #[must_use]
    pub fn id(&self) -> &JobId {
        &self.id
    }

    #[must_use]
    pub fn pipeline_id(&self) -> &PipelineId {
        &self.pipeline_id
    }

    #[must_use]
    pub fn state(&self) -> &JobState {
        &self.state
    }

    #[must_use]
    pub fn status(&self) -> JobStatus {
        match &self.state {
            JobState::Pending => JobStatus::Pending,
            JobState::Running { .. } => JobStatus::Running,
            JobState::Terminal { outcome, .. } => match outcome {
                TerminalOutcome::Completed => JobStatus::Completed,
                TerminalOutcome::Failed => JobStatus::Failed,
                TerminalOutcome::Cancelled => JobStatus::Cancelled,
                TerminalOutcome::Orphaned => JobStatus::Orphaned,
            },
        }
    }

    #[must_use]
    pub fn nodes(&self) -> &[PipelineNode] {
        &self.nodes
    }

    #[must_use]
    pub fn node_executions(&self) -> &[JobNode] {
        &self.node_executions
    }

    #[must_use]
    pub fn inputs(&self) -> &[(EnvKey, EnvValue)] {
        &self.inputs
    }

    #[must_use]
    pub fn agent_app_id(&self) -> Option<&AppId> {
        self.agent_app_id.as_ref()
    }

    #[must_use]
    pub fn origin(&self) -> &JobOrigin {
        &self.origin
    }

    #[must_use]
    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    #[must_use]
    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }

    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }

    #[must_use]
    pub fn started_at(&self) -> Option<DateTime<Utc>> {
        match &self.state {
            JobState::Pending => None,
            JobState::Running { started_at } => Some(*started_at),
            JobState::Terminal { started_at, .. } => *started_at,
        }
    }

    #[must_use]
    pub fn finished_at(&self) -> Option<DateTime<Utc>> {
        match &self.state {
            JobState::Pending | JobState::Running { .. } => None,
            JobState::Terminal { finished_at, .. } => Some(*finished_at),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::{ProjectId, UserId};
    use crate::domain::pipeline::PipelineName;
    use chrono::Duration;

    fn node_id(s: &str) -> NodeId {
        NodeId::new(s).unwrap()
    }

    fn make_job(pipeline: &Pipeline) -> Job {
        Job::create_from_pipeline(
            pipeline,
            JobOrigin::Human {
                user_id: UserId::generate(),
            },
        )
    }

    fn running_job(pipeline: &Pipeline) -> Job {
        make_job(pipeline).start(clock::now()).unwrap()
    }

    fn make_pipeline(nodes: Vec<PipelineNode>) -> Pipeline {
        Pipeline::create(
            PipelineName::new("test").unwrap(),
            ProjectId::generate(),
            nodes,
        )
        .unwrap()
    }

    fn action(id: &str, deps: &[&str]) -> PipelineNode {
        use crate::domain::pipeline::Step;
        PipelineNode::new(
            node_id(id),
            deps.iter().map(|d| node_id(d)).collect(),
            Step::exec("echo".into(), vec![]).unwrap(),
            None,
            vec![],
        )
    }

    fn state_of(job: &Job, id: &str) -> NodeState {
        job.find_execution(&node_id(id)).unwrap().state()
    }

    #[test]
    fn creates_job_from_pipeline() {
        let pipeline = make_pipeline(vec![action("a", &[]), action("b", &["a"])]);
        let job = make_job(&pipeline);

        assert_eq!(job.status(), JobStatus::Pending);
        assert_eq!(job.node_executions().len(), 2);
        assert_eq!(job.pipeline_id(), pipeline.id());
        assert!(job.started_at().is_none());
        assert_eq!(job.version(), 0);
    }

    #[test]
    fn a_job_keeps_the_nodes_of_its_pipeline_at_creation() {
        let mut pipeline = make_pipeline(vec![action("a", &[]), action("b", &["a"])]);
        let job = make_job(&pipeline);

        pipeline.update_nodes(vec![action("new", &[])]).unwrap();

        let ids: Vec<&str> = job.nodes().iter().map(|n| n.id().as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
        let executions: Vec<&str> = job
            .node_executions()
            .iter()
            .map(|n| n.node_id().as_str())
            .collect();
        assert_eq!(executions, ids);
    }

    #[test]
    fn every_transition_takes_its_time_from_the_caller() {
        let pipeline = make_pipeline(vec![action("a", &[])]);
        let t0 = clock::now() - Duration::minutes(5);
        let t1 = t0 + Duration::seconds(1);
        let t2 = t0 + Duration::seconds(2);

        let job = make_job(&pipeline)
            .start(t0)
            .unwrap()
            .apply_node_started(&node_id("a"), t1)
            .unwrap()
            .apply_node_finished(&node_id("a"), NodeOutcome::Completed, t2)
            .unwrap()
            .complete(t2)
            .unwrap();

        assert_eq!(job.started_at(), Some(t0));
        assert_eq!(job.finished_at(), Some(t2));
        assert_eq!(job.updated_at(), t2);
        let node = job.find_execution(&node_id("a")).unwrap();
        assert_eq!(node.started_at(), Some(t1));
        assert_eq!(node.finished_at(), Some(t2));
    }

    #[test]
    fn start_transitions_to_running() {
        let pipeline = make_pipeline(vec![action("a", &[])]);
        let job = make_job(&pipeline);
        assert_eq!(job.status(), JobStatus::Pending);

        let job = job.start(clock::now()).unwrap();
        assert_eq!(job.status(), JobStatus::Running);
        assert!(job.started_at().is_some());

        assert!(job.start(clock::now()).is_err());
    }

    #[test]
    fn complete_and_fail_end_a_running_job() {
        let pipeline = make_pipeline(vec![action("a", &[])]);
        let completed = running_job(&pipeline).complete(clock::now()).unwrap();
        let failed = running_job(&pipeline).fail(clock::now()).unwrap();

        assert_eq!(completed.status(), JobStatus::Completed);
        assert_eq!(failed.status(), JobStatus::Failed);
        assert!(completed.finished_at().is_some());
    }

    #[test]
    fn only_completion_needs_a_started_job() {
        let pipeline = make_pipeline(vec![action("a", &[])]);
        let now = clock::now();

        assert!(make_job(&pipeline).complete(now).is_err());
        for ended in [
            make_job(&pipeline).fail(now).unwrap(),
            make_job(&pipeline).cancel(now).unwrap(),
            make_job(&pipeline).orphan(now).unwrap(),
        ] {
            assert!(ended.is_terminal());
            assert!(ended.started_at().is_none());
            assert_eq!(ended.finished_at(), Some(now));
            assert_eq!(state_of(&ended, "a"), NodeState::Cancelled);
        }
    }

    #[test]
    fn every_end_closes_the_active_nodes_and_keeps_the_finished_ones() {
        let pipeline = make_pipeline(vec![
            action("done", &[]),
            action("busy", &[]),
            action("waiting", &["done"]),
        ]);
        let now = clock::now();
        let midway = || {
            running_job(&pipeline)
                .apply_node_started(&node_id("done"), now)
                .unwrap()
                .apply_node_finished(&node_id("done"), NodeOutcome::Completed, now)
                .unwrap()
                .apply_node_started(&node_id("busy"), now)
                .unwrap()
        };

        for ended in [
            midway().complete(now).unwrap(),
            midway().fail(now).unwrap(),
            midway().cancel(now).unwrap(),
            midway().orphan(now).unwrap(),
        ] {
            assert_eq!(state_of(&ended, "done"), NodeState::Completed);
            assert_eq!(state_of(&ended, "busy"), NodeState::Cancelled);
            assert_eq!(state_of(&ended, "waiting"), NodeState::Cancelled);
            let busy = ended.find_execution(&node_id("busy")).unwrap();
            assert!(busy.started_at().is_some());
            assert_eq!(busy.finished_at(), Some(now));
        }
    }

    #[test]
    fn an_ended_job_refuses_every_transition() {
        let pipeline = make_pipeline(vec![action("a", &[])]);
        let now = clock::now();
        let ended = || running_job(&pipeline).cancel(now).unwrap();

        assert!(ended().start(now).is_err());
        assert!(ended().complete(now).is_err());
        assert!(ended().fail(now).is_err());
        assert!(ended().cancel(now).is_err());
        assert!(ended().orphan(now).is_err());
        assert!(ended().apply_node_started(&node_id("a"), now).is_err());
        assert!(ended().apply_node_skipped(&node_id("a"), now).is_err());
    }

    #[test]
    fn a_node_event_needs_a_running_job() {
        let pipeline = make_pipeline(vec![action("a", &[])]);
        let now = clock::now();

        assert!(
            make_job(&pipeline)
                .apply_node_started(&node_id("a"), now)
                .is_err()
        );
        assert!(
            make_job(&pipeline)
                .apply_node_skipped(&node_id("a"), now)
                .is_err()
        );
        let orphaned = running_job(&pipeline)
            .apply_node_started(&node_id("a"), now)
            .unwrap()
            .orphan(now)
            .unwrap();
        assert!(
            orphaned
                .apply_node_finished(&node_id("a"), NodeOutcome::Completed, now)
                .is_err()
        );
    }

    #[test]
    fn a_job_is_live_only_on_its_agent_until_it_ends() {
        let pipeline = make_pipeline(vec![action("a", &[])]);
        let agent = AppId::new("agent-1");
        let placed = Job::from_persistence(
            JobId::generate(),
            pipeline.id().clone(),
            JobState::Pending,
            Some(agent.clone()),
            pipeline.nodes().to_vec(),
            vec![JobNode::new(node_id("a"))],
            Vec::new(),
            JobOrigin::Human {
                user_id: UserId::generate(),
            },
            clock::now(),
            clock::now(),
            3,
        );

        assert!(placed.ensure_live_on(&agent).is_ok());
        assert!(matches!(
            placed.ensure_live_on(&AppId::new("agent-2")),
            Err(DomainError::BusinessRule(_))
        ));
        assert!(make_job(&pipeline).ensure_live_on(&agent).is_err());
        assert!(
            placed
                .cancel(clock::now())
                .unwrap()
                .ensure_live_on(&agent)
                .is_err()
        );
    }

    #[test]
    fn apply_node_started_sets_running() {
        let pipeline = make_pipeline(vec![action("a", &[])]);
        let now = clock::now();
        let job = running_job(&pipeline)
            .apply_node_started(&node_id("a"), now)
            .unwrap();

        let exec = job.find_execution(&node_id("a")).unwrap();
        assert_eq!(exec.state(), NodeState::Running);
        assert!(exec.started_at().is_some());
    }

    #[test]
    fn apply_node_finished_sets_terminal() {
        let pipeline = make_pipeline(vec![action("a", &[])]);
        let now = clock::now();
        let job = running_job(&pipeline)
            .apply_node_started(&node_id("a"), now)
            .unwrap()
            .apply_node_finished(&node_id("a"), NodeOutcome::Completed, now)
            .unwrap();

        let exec = job.find_execution(&node_id("a")).unwrap();
        assert_eq!(exec.state(), NodeState::Completed);
        assert!(exec.finished_at().is_some());
    }

    #[test]
    fn logs_readable_for_gates_pending_nodes() {
        let pipeline = make_pipeline(vec![action("a", &[]), action("b", &["a"])]);
        let mut job = running_job(&pipeline);

        assert!(!job.logs_readable_for(&node_id("a")));
        assert!(!job.logs_readable_for(&node_id("b")));

        assert!(!job.logs_readable_for(&node_id("ghost")));

        job = job.apply_node_started(&node_id("a"), clock::now()).unwrap();
        assert!(job.logs_readable_for(&node_id("a")));

        job = job
            .apply_node_finished(&node_id("a"), NodeOutcome::Completed, clock::now())
            .unwrap();
        assert!(job.logs_readable_for(&node_id("a")));

        assert!(!job.logs_readable_for(&node_id("b")));
    }

    #[test]
    fn cannot_finish_pending_node() {
        let pipeline = make_pipeline(vec![action("a", &[])]);
        let job = running_job(&pipeline);

        assert!(
            job.apply_node_finished(&node_id("a"), NodeOutcome::Completed, clock::now())
                .is_err()
        );
    }

    #[test]
    fn cannot_start_nonexistent_node() {
        let pipeline = make_pipeline(vec![action("a", &[])]);
        let job = running_job(&pipeline);

        assert!(job.apply_node_started(&node_id("z"), clock::now()).is_err());
    }

    #[test]
    fn apply_node_skipped_from_pending() {
        let pipeline = make_pipeline(vec![action("a", &[]), action("b", &["a"])]);
        let job = running_job(&pipeline)
            .apply_node_skipped(&node_id("b"), clock::now())
            .unwrap();

        let exec = job.find_execution(&node_id("b")).unwrap();
        assert_eq!(exec.state(), NodeState::Skipped);
        assert!(exec.finished_at().is_some());
    }

    #[test]
    fn apply_node_skipped_from_running() {
        let pipeline = make_pipeline(vec![action("a", &[])]);
        let job = running_job(&pipeline)
            .apply_node_started(&node_id("a"), clock::now())
            .unwrap()
            .apply_node_skipped(&node_id("a"), clock::now())
            .unwrap();

        assert_eq!(state_of(&job, "a"), NodeState::Skipped);
    }

    #[test]
    fn cannot_skip_terminal_node() {
        let pipeline = make_pipeline(vec![action("a", &[])]);
        let now = clock::now();
        let job = running_job(&pipeline)
            .apply_node_started(&node_id("a"), now)
            .unwrap()
            .apply_node_finished(&node_id("a"), NodeOutcome::Completed, now)
            .unwrap();

        assert!(job.apply_node_skipped(&node_id("a"), now).is_err());
    }

    #[test]
    fn is_terminal_reflects_status() {
        let pipeline = make_pipeline(vec![action("a", &[])]);
        let job = make_job(&pipeline);
        assert!(!job.is_terminal());

        let job = job.start(clock::now()).unwrap();
        assert!(!job.is_terminal());

        let job = job.complete(clock::now()).unwrap();
        assert!(job.is_terminal());
    }

    /// Golden: the variant and field names are the on-disk format of `jobs.node_executions`.
    #[test]
    fn job_nodes_jsonb_shape_is_stable() {
        const STORED: &str = r#"[
            {"node_id":"a","execution":{"state":"pending"}},
            {"node_id":"b","execution":{"state":"running","started_at":"2026-01-15T10:30:00Z"}},
            {"node_id":"c","execution":{"state":"finished","started_at":"2026-01-15T10:30:00Z",
             "finished_at":"2026-01-15T10:30:00Z","outcome":"completed"}},
            {"node_id":"d","execution":{"state":"finished","started_at":null,
             "finished_at":"2026-01-15T10:30:00Z","outcome":"skipped"}}
        ]"#;

        let nodes: Vec<JobNode> = serde_json::from_str(STORED).unwrap();
        assert_eq!(nodes.len(), 4);
        assert!(matches!(nodes[0].execution(), NodeExecution::Pending));
        assert!(matches!(
            nodes[1].execution(),
            NodeExecution::Running { .. }
        ));
        assert!(matches!(
            nodes[2].execution(),
            NodeExecution::Finished {
                outcome: NodeOutcome::Completed,
                started_at: Some(_),
                ..
            }
        ));
        assert!(matches!(
            nodes[3].execution(),
            NodeExecution::Finished {
                outcome: NodeOutcome::Skipped,
                started_at: None,
                ..
            }
        ));

        let round_tripped: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&nodes).unwrap()).unwrap();
        let original: serde_json::Value = serde_json::from_str(STORED).unwrap();
        assert_eq!(round_tripped, original, "serialized shape drifted");
    }
}
