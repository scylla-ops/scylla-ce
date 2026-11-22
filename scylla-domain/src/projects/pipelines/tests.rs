//! Unit tests of the checks of the nodes and of their graph.

use super::{
    Case, Combinator, EnvValue, EnvVar, Need, Node, NodeId, Nodes, Operand, PortName, Rule,
    Runtime, Step, Switch, Test,
};
use crate::DomainError;

/// Reads a need: `node` or `node:port`.
pub(super) fn need(text: &str) -> Need {
    let (node, port) = text
        .split_once(':')
        .map_or((text, None), |(node, port)| (node, Some(port)));
    Need {
        node: NodeId::parse(node.to_owned()).unwrap(),
        port: port.map(|port| PortName::parse(port.to_owned()).unwrap()),
    }
}

/// Makes a node that needs other nodes, each as `node` or `node:port`.
pub(super) fn node(id: &str, needs: &[&str]) -> Node {
    Node {
        id: NodeId::parse(id.to_owned()).unwrap(),
        needs: needs.iter().map(|text| need(text)).collect(),
        step: Step::Exec(vec!["make".to_owned()]),
        env: Vec::new(),
        timeout_seconds: 0,
        retries: 0,
        working_dir: None,
        runtime: Runtime::Process,
    }
}

/// Returns the path and the rule of the refusal of some nodes.
pub(super) fn refusal(nodes: Vec<Node>) -> (String, DomainError) {
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
        ("nodes[0].needs[0].node", DomainError::UnknownNode),
        ("nodes[0].needs[0].node", DomainError::UnknownNode),
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

/// Returns a port.
pub(super) fn port(name: &str) -> PortName {
    PortName::parse(name.to_owned()).unwrap()
}

/// Makes a rule with a literal on the right, if any.
pub(super) fn rule(left: Operand, test: Test, right: Option<&str>) -> Rule {
    let right = right.map(|text| Operand::Literal(text.to_owned()));
    Rule { left, test, right }
}

/// Returns a rule that compares a variable with a text.
pub(super) fn on_var() -> Rule {
    rule(
        Operand::Var("BRANCH".to_owned()),
        Test::Equals,
        Some("main"),
    )
}

/// Returns an output operand.
pub(super) fn output(node: &str, key: &str) -> Operand {
    let node = NodeId::parse(node.to_owned()).unwrap();
    Operand::Output {
        node,
        key: key.to_owned(),
    }
}

/// Makes a case of a port and its rules.
pub(super) fn case(name: &str, rules: Vec<Rule>) -> Case {
    Case {
        port: port(name),
        combinator: Combinator::All,
        rules,
    }
}

/// Makes a switch node with these cases and the otherwise port `no`.
pub(super) fn switch_of(id: &str, needs: &[&str], cases: Vec<Case>) -> Node {
    let mut node = node(id, needs);
    node.step = Step::Switch(Switch {
        cases,
        otherwise: Some(port("no")),
        all_matches: false,
    });
    node
}

/// Makes a switch node with one case `yes` of these rules, and the otherwise port `no`.
pub(super) fn switch(id: &str, needs: &[&str], rules: Vec<Rule>) -> Node {
    switch_of(id, needs, vec![case("yes", rules)])
}

/// Makes a merge node.
pub(super) fn merge(id: &str, needs: &[&str]) -> Node {
    let mut node = node(id, needs);
    node.step = Step::Merge;
    node
}

/// The path and the rule of a refusal.
pub(super) type Refusal = (String, DomainError);

/// Returns the refusal of each case of nodes, next to the refusal that the case expects.
pub(super) fn refusals(cases: Vec<(Vec<Node>, &str, DomainError)>) -> Vec<(Refusal, Refusal)> {
    cases
        .into_iter()
        .map(|(nodes, path, error)| (refusal(nodes), (path.to_owned(), error)))
        .collect()
}

/// A switch, a merge, an `error` need, an output two levels up, a `SCYLLA_` variable, and two
/// needs of one node with two ports pass.
#[test]
fn a_graph_with_conditions_passes() {
    // given
    let rules = vec![
        rule(output("build", "BRANCH"), Test::Equals, Some("main")),
        rule(
            Operand::Var("SCYLLA_JOB_ID".to_owned()),
            Test::NotEmpty,
            None,
        ),
    ];
    let nodes = vec![
        node("build", &[]),
        node("test", &["build"]),
        switch("is-main", &["test"], rules),
        node("deploy", &["build", "is-main:yes"]),
        node("notify", &["test:error"]),
        merge("done", &["deploy", "is-main:no"]),
        merge("always", &["test", "test:error"]),
    ];

    // when
    let parsed = Nodes::parse(nodes);

    // then
    assert_eq!(parsed.map(|nodes| nodes.as_slice().len()), Ok(7));
}

/// A port must be a port of its node, and a pair of a node and a port comes one time.
#[test]
fn a_bad_port_is_refused() {
    // given
    let gate = || switch("s", &["a"], vec![on_var()]);

    // when
    let refusals = refusals(vec![
        (
            vec![node("a", &[]), gate(), node("b", &["s:maybe"])],
            "nodes[2].needs[0].port",
            DomainError::UnknownPort,
        ),
        (
            vec![node("a", &[]), gate(), node("b", &["s:error"])],
            "nodes[2].needs[0].port",
            DomainError::UnknownPort,
        ),
        (
            vec![node("a", &[]), node("b", &["a:done"])],
            "nodes[1].needs[0].port",
            DomainError::UnknownPort,
        ),
        (
            vec![node("a", &[]), merge("m", &["a"]), node("b", &["m:yes"])],
            "nodes[2].needs[0].port",
            DomainError::UnknownPort,
        ),
        (
            vec![node("a", &[]), node("b", &["a:error", "a:error"])],
            "nodes[1].needs[1]",
            DomainError::Duplicate,
        ),
    ]);

    // then
    for (refused, expected) in refusals {
        assert_eq!(refused, expected);
    }
}
