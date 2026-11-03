//! Unit tests of the checks of the nodes and of their graph.

use super::{EnvValue, EnvVar, Node, NodeId, Nodes, Runtime, Step};
use crate::DomainError;

/// Makes a node that needs other nodes.
fn node(id: &str, needs: &[&str]) -> Node {
    Node {
        id: NodeId::parse(id.to_owned()).unwrap(),
        needs: needs
            .iter()
            .map(|need| NodeId::parse((*need).to_owned()).unwrap())
            .collect(),
        step: Step::Exec(vec!["make".to_owned()]),
        env: Vec::new(),
        timeout_seconds: 0,
        retries: 0,
        working_dir: None,
        runtime: Runtime::Process,
    }
}

/// Returns the path and the rule of the refusal of some nodes.
fn refusal(nodes: Vec<Node>) -> (String, DomainError) {
    let error = Nodes::parse(nodes).unwrap_err();
    (error.path, error.error)
}

/// A graph with no cycle passes, and a timeout of 0 gets the default.
#[test]
fn a_graph_with_no_cycle_passes() {
    // given
    let nodes = vec![node("a", &[]), node("b", &["a"]), node("c", &["a", "b"])];

    // when
    let parsed = Nodes::parse(nodes).unwrap();

    // then
    assert_eq!(parsed.as_slice()[0].timeout_seconds, Node::DEFAULT_TIMEOUT);
    assert!(!parsed.need_container());
}

/// A cycle, an unknown need, a need of the node itself, two needs of one node, two nodes with
/// one id, and no node are refused.
#[test]
fn a_bad_graph_is_refused() {
    // given
    let cycle = vec![node("a", &["c"]), node("b", &["a"]), node("c", &["b"])];
    let unknown = vec![node("a", &["z"])];
    let itself = vec![node("a", &["a"])];
    let twice = vec![node("a", &[]), node("b", &["a", "a"])];
    let same = vec![node("a", &[]), node("a", &[])];

    // when
    let refusals = [cycle, unknown, itself, twice, same, Vec::new()].map(refusal);

    // then
    let expected = [
        ("nodes", DomainError::Cycle),
        ("nodes[0].needs[0]", DomainError::UnknownNode),
        ("nodes[0].needs[0]", DomainError::UnknownNode),
        ("nodes[1].needs[1]", DomainError::Duplicate),
        ("nodes[1].id", DomainError::Duplicate),
        ("nodes", DomainError::Empty),
    ];
    for ((path, error), (expected_path, expected_error)) in refusals.into_iter().zip(expected) {
        assert_eq!((path.as_str(), error), (expected_path, expected_error));
    }
}

/// The checks of a node: a reserved or a second variable, a path out of the work folder, a
/// long timeout, and an empty command.
#[test]
fn a_bad_node_is_refused() {
    // given
    let var = |name: &str| EnvVar {
        name: name.to_owned(),
        value: EnvValue::Literal("1".to_owned()),
    };
    let mut reserved = node("a", &[]);
    reserved.env = vec![var("scylla_token")];
    let mut twice = node("a", &[]);
    twice.env = vec![var("A"), var("A")];
    let mut escape = node("a", &[]);
    escape.working_dir = Some("src/../../etc".to_owned());
    let mut long = node("a", &[]);
    long.timeout_seconds = Node::MAX_TIMEOUT + 1;
    let mut empty = node("a", &[]);
    empty.step = Step::Exec(Vec::new());

    // when
    let refusals = [reserved, twice, escape, long, empty].map(|node| refusal(vec![node]));

    // then
    let paths: Vec<_> = refusals.iter().map(|(path, _)| path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "nodes[0].env[0].name",
            "nodes[0].env[1].name",
            "nodes[0].working_dir",
            "nodes[0].timeout_seconds",
            "nodes[0].exec",
        ]
    );
    assert_eq!(refusals[0].1, DomainError::ReservedName);
}
