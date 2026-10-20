use bon::bon;
use chrono::{DateTime, Utc};

use crate::domain::clock;
use crate::domain::ids::{AppId, JobId, UserId};
use crate::domain::job::{Job, JobNode, JobState, NodeExecution};
use crate::domain::job::{JobOrigin, JobStatus};
use crate::domain::pipeline::Pipeline;

pub struct JobBuilder;

fn default_origin() -> JobOrigin {
    JobOrigin::Human {
        user_id: UserId::generate(),
    }
}

#[bon]
#[allow(clippy::new_ret_no_self, clippy::must_use_candidate)]
impl JobBuilder {
    #[builder(start_fn = new, finish_fn = build)]
    pub fn assemble(
        #[builder(start_fn)] pipeline: &Pipeline,
        id: Option<JobId>,
        #[builder(default = JobStatus::Pending)] status: JobStatus,
        created_at: Option<DateTime<Utc>>,
        updated_at: Option<DateTime<Utc>>,
        started_at: Option<DateTime<Utc>>,
        finished_at: Option<DateTime<Utc>>,
        #[builder(default = false)] running: bool,
        terminated: Option<JobStatus>,
        #[builder(default = default_origin())] origin: JobOrigin,
        agent: Option<AppId>,
        #[builder(default = 0)] version: u64,
    ) -> Job {
        let pipeline_id = pipeline.id().clone();
        let node_executions: Vec<JobNode> = pipeline
            .nodes()
            .iter()
            .map(|n| JobNode::from_persistence(n.id().clone(), NodeExecution::Pending))
            .collect();

        let (status, started_at, finished_at) = if let Some(terminal) = terminated {
            let now = clock::now();
            (
                terminal,
                Some(now - chrono::Duration::seconds(1)),
                Some(now),
            )
        } else if running {
            (JobStatus::Running, Some(clock::now()), finished_at)
        } else {
            (status, started_at, finished_at)
        };
        let state = JobState::from_columns(status, started_at, finished_at)
            .expect("test fixture built an inconsistent job state");

        let now = created_at.unwrap_or_else(clock::now);
        Job::from_persistence(
            id.unwrap_or_else(JobId::generate),
            pipeline_id,
            state,
            agent,
            pipeline.nodes().to_vec(),
            node_executions,
            Vec::new(),
            origin,
            now,
            updated_at.unwrap_or(now),
            version,
        )
    }
}

#[must_use]
pub fn job(pipeline: &Pipeline) -> Job {
    JobBuilder::new(pipeline).build()
}

/// The row a store holds after a write of `job` that sets its agent: same state, next version.
#[must_use]
pub fn stored(job: &Job, agent: Option<&AppId>) -> Job {
    Job::from_persistence(
        job.id().clone(),
        job.pipeline_id().clone(),
        job.state().clone(),
        agent.cloned(),
        job.nodes().to_vec(),
        job.node_executions().to_vec(),
        job.inputs().to_vec(),
        job.origin().clone(),
        job.created_at(),
        job.updated_at(),
        job.version() + 1,
    )
}
