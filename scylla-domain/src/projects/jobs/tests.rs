//! Unit tests of the state machine of a job.

use time::OffsetDateTime;

use super::{Job, JobEvent, JobStatus, NodeRun, NodeStatus};
use crate::identity::actors::ActorId;
use crate::projects::pipelines::{
    Case, Combinator, Node, NodeId, Operand, PortName, Rule, Runtime, Step, Switch, Test,
};
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
        trigger_id: None,
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
            ports: Vec::new(),
        },
        JobEvent::NodeStarted(id("test")),
        JobEvent::NodeCompleted {
            node: id("test"),
            exit_code: Some(0),
            ports: Vec::new(),
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
            ports: Vec::new(),
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

/// Makes a running job with a switch of the ports `a`, `b` and `other`, and a node `build`.
fn switch_job(all_matches: bool) -> Job {
    let port = |name: &str| PortName::parse(name.to_owned()).unwrap();
    let case = |name: &str| Case {
        port: port(name),
        combinator: Combinator::Any,
        rules: vec![Rule {
            left: Operand::Var("A".to_owned()),
            test: Test::NotEmpty,
            right: None,
        }],
    };
    let mut gate = node("gate");
    gate.retries = 0;
    gate.step = Step::Switch(Switch {
        cases: vec![case("a"), case("b")],
        otherwise: Some(port("other")),
        all_matches,
    });
    let mut job = job();
    job.nodes = vec![node("build"), gate];
    job.runs = job.nodes.iter().map(NodeRun::pending).collect();
    let now = OffsetDateTime::now_utc();
    job.apply(&JobEvent::JobStarted, now).unwrap();
    for name in ["build", "gate"] {
        job.apply(&JobEvent::NodeStarted(id(name)), now).unwrap();
    }
    job
}

/// Returns the result of a completion of a node with some ports.
fn complete(job: &mut Job, node: &str, ports: &[&str]) -> Result<(), DomainError> {
    let ports = ports
        .iter()
        .map(|port| PortName::parse((*port).to_owned()).unwrap())
        .collect();
    let event = JobEvent::NodeCompleted {
        node: id(node),
        exit_code: None,
        ports,
    };
    job.apply(&event, OffsetDateTime::now_utc())
}

/// A switch completes with its own ports, each one time, and one port at most with no
/// `all_matches`. A node that is not a switch has no port.
#[test]
fn a_completion_must_fit_the_ports_of_the_switch() {
    // given
    let mut one = switch_job(false);
    let mut all = switch_job(true);

    // when
    let refused = [
        complete(&mut one.clone(), "build", &["a"]),
        complete(&mut one.clone(), "gate", &["c"]),
        complete(&mut one.clone(), "gate", &["a", "b"]),
        complete(&mut all.clone(), "gate", &["a", "a"]),
    ];
    let first = complete(&mut one, "gate", &["other"]);
    let both = complete(&mut all, "gate", &["a", "b"]);

    // then
    for result in refused {
        assert_eq!(result, Err(DomainError::NotAllowed));
    }
    assert!(first.is_ok() && both.is_ok());
    assert_eq!(
        one.runs[1].ports,
        vec![PortName::parse("other".to_owned()).unwrap()]
    );
    assert_eq!(
        (all.runs[1].status, all.runs[1].ports.len()),
        (NodeStatus::Completed, 2)
    );
}
