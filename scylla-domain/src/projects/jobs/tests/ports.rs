//! Unit tests of the ports of a switch in a job.

use time::OffsetDateTime;

use super::{id, job, node};
use crate::projects::jobs::{Job, JobEvent, NodeRun, NodeStatus};
use crate::projects::pipelines::{Case, Combinator, Operand, PortName, Rule, Step, Switch, Test};
use crate::DomainError;

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
