//! Unit tests of the state machine of a job.

use time::OffsetDateTime;

use super::{Job, JobEvent, JobStatus, NodeRun, NodeStatus};
use crate::identity::actors::ActorId;
use crate::projects::pipelines::{Node, NodeId, Runtime, Step};
use crate::DomainError;

/// Makes a node with one retry.
fn node(id: &str) -> Node {
    Node {
        id: NodeId::parse(id.to_owned()).unwrap(),
        needs: Vec::new(),
        step: Step::Exec(vec!["true".to_owned()]),
        env: Vec::new(),
        timeout_seconds: 60,
        retries: 1,
        working_dir: None,
        runtime: Runtime::Process,
    }
}

/// Makes a pending job of two nodes.
fn job() -> Job {
    let nodes = vec![node("build"), node("test")];
    Job {
        id: super::JobId::generate(),
        pipeline_id: crate::Id::generate(),
        project_id: crate::Id::generate(),
        workspace_id: crate::Id::generate(),
        created_by: ActorId::generate(),
        rerun_of: None,
        status: JobStatus::Pending,
        runs: nodes.iter().map(NodeRun::pending).collect(),
        nodes,
        agent_id: None,
        error: None,
        version: 1,
        created_at: OffsetDateTime::now_utc(),
        started_at: None,
        finished_at: None,
    }
}

/// Returns the id of a node.
fn id(name: &str) -> NodeId {
    NodeId::parse(name.to_owned()).unwrap()
}

/// A job runs its nodes, retries one time, and completes when each node ended.
#[test]
fn a_job_runs_to_completion() {
    // given
    let mut job = job();
    let now = OffsetDateTime::now_utc();
    let events = [
        JobEvent::JobStarted,
        JobEvent::NodeStarted(id("build")),
        JobEvent::NodeRetrying {
            node: id("build"),
            attempt: 2,
        },
        JobEvent::NodeCompleted {
            node: id("build"),
            exit_code: Some(0),
        },
        JobEvent::NodeStarted(id("test")),
        JobEvent::NodeCompleted {
            node: id("test"),
            exit_code: Some(0),
        },
    ];

    // when
    let applied: Result<Vec<_>, _> = events.iter().map(|event| job.apply(event, now)).collect();
    let early = job.clone().apply(&JobEvent::JobStarted, now);
    let completed = job.apply(&JobEvent::JobCompleted, now);

    // then
    assert!(applied.is_ok() && completed.is_ok());
    assert_eq!(early, Err(DomainError::NotAllowed));
    assert_eq!(job.status, JobStatus::Completed);
    assert_eq!(job.runs[0].attempt, 2);
}

/// An event that does not fit is refused: a third attempt, a node of a pending job, a
/// completion with a node that runs, an event after the end.
#[test]
fn an_event_must_fit_the_state() {
    // given
    let mut job = job();
    let now = OffsetDateTime::now_utc();
    let early = job.apply(&JobEvent::NodeStarted(id("build")), now);
    job.apply(&JobEvent::JobStarted, now).unwrap();
    job.apply(&JobEvent::NodeStarted(id("build")), now).unwrap();
    job.apply(
        &JobEvent::NodeRetrying {
            node: id("build"),
            attempt: 2,
        },
        now,
    )
    .unwrap();

    // when
    let third = job.apply(
        &JobEvent::NodeRetrying {
            node: id("build"),
            attempt: 3,
        },
        now,
    );
    let unknown = job.apply(&JobEvent::NodeStarted(id("deploy")), now);
    let completed = job.apply(&JobEvent::JobCompleted, now);
    job.apply(&JobEvent::JobFailed("disk full".to_owned()), now)
        .unwrap();
    let after = job.apply(&JobEvent::NodeStarted(id("test")), now);

    // then
    for result in [early, third, unknown, completed, after] {
        assert_eq!(result, Err(DomainError::NotAllowed));
    }
    assert_eq!(job.status, JobStatus::Failed);
    assert_eq!(job.error.as_deref(), Some("disk full"));
    assert!(job
        .runs
        .iter()
        .all(|run| run.status == NodeStatus::Cancelled));
}

/// The end of a job cancels the nodes that did not end, and a final job does not change.
#[test]
fn an_end_cancels_the_open_nodes() {
    // given
    let mut job = job();
    let now = OffsetDateTime::now_utc();
    job.apply(&JobEvent::JobStarted, now).unwrap();
    job.apply(&JobEvent::NodeStarted(id("build")), now).unwrap();
    job.apply(
        &JobEvent::NodeCompleted {
            node: id("build"),
            exit_code: Some(0),
        },
        now,
    )
    .unwrap();

    // when
    let cancelled = job.end(JobStatus::Cancelled, None, now);
    let again = job.end(JobStatus::Orphaned, None, now);

    // then
    assert!(cancelled && !again);
    assert_eq!(job.status, JobStatus::Cancelled);
    let states: Vec<_> = job.runs.iter().map(|run| run.status).collect();
    assert_eq!(states, vec![NodeStatus::Completed, NodeStatus::Cancelled]);
}
