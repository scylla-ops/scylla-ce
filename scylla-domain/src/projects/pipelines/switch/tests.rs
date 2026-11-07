//! Unit tests of the checks of the switches, of the merges and of the ports of the needs.

use super::super::tests::{case, merge, node, on_var, output, port, refusal, refusals, rule};
use super::super::tests::{switch, switch_of};
use super::{Operand, Test};
use crate::projects::pipelines::{EnvValue, EnvVar, Node, Runtime, Step};
use crate::DomainError;

/// The size of a switch, its ports (unique, and not `error`), and its fields are checked.
#[test]
fn a_bad_switch_is_refused() {
    // given
    let many: Vec<_> = (0..17)
        .map(|i| case(&format!("p{i}"), vec![on_var()]))
        .collect();
    let same = vec![case("a", vec![on_var()]); 2];
    let mut otherwise = switch("s", &[], vec![on_var()]);
    if let Step::Switch(switch) = &mut otherwise.step {
        switch.otherwise = Some(port("yes"));
    }
    let mut otherwise_error = switch("s", &[], vec![on_var()]);
    if let Step::Switch(switch) = &mut otherwise_error.step {
        switch.otherwise = Some(port("error"));
    }
    let case_error = switch_of("s", &[], vec![case("error", vec![on_var()])]);
    let mut retries = switch("s", &[], vec![on_var()]);
    retries.retries = 1;
    let mut work = switch("s", &[], vec![on_var()]);
    work.working_dir = Some("src".to_owned());
    let mut container = switch("s", &[], vec![on_var()]);
    container.runtime = Runtime::Container {
        image: "alpine".to_owned(),
    };
    let (cases, rules) = ("nodes[0].switch.cases", "nodes[0].switch.cases[0].rules");

    // when
    let nodes = [
        switch_of("s", &[], Vec::new()),
        switch_of("s", &[], many),
        switch("s", &[], Vec::new()),
        switch("s", &[], vec![on_var(); 33]),
        switch_of("s", &[], same),
        otherwise,
        case_error,
        otherwise_error,
        retries,
        work,
        container,
    ];
    let refusals = nodes.map(|node| refusal(vec![node]));

    // then
    let expected = [
        (cases, DomainError::Empty),
        (cases, DomainError::TooMany { max: 16 }),
        (rules, DomainError::Empty),
        (rules, DomainError::TooMany { max: 32 }),
        ("nodes[0].switch.cases[1].port", DomainError::Duplicate),
        ("nodes[0].switch.otherwise", DomainError::Duplicate),
        ("nodes[0].switch.cases[0].port", DomainError::ReservedPort),
        ("nodes[0].switch.otherwise", DomainError::ReservedPort),
        ("nodes[0].retries", DomainError::NotExpected),
        ("nodes[0].working_dir", DomainError::NotExpected),
        ("nodes[0].container", DomainError::NotExpected),
    ];
    for ((path, error), (expected_path, expected_error)) in refusals.into_iter().zip(expected) {
        assert_eq!((path.as_str(), error), (expected_path, expected_error));
    }
}

/// A rule needs a right value except for an empty test, a number in a number test, a text in
/// its limit, and names of the form of a variable.
#[test]
fn a_bad_rule_is_refused() {
    // given
    let var = |name: &str| Operand::Var(name.to_owned());
    let large = Operand::Literal("x".repeat(Node::MAX_TEXT + 1));
    let (number, name) = (DomainError::NotANumber, DomainError::NotAnEnvName);
    let rules = [
        (
            rule(var("A"), Test::Equals, None),
            "right",
            DomainError::Required,
        ),
        (
            rule(var("A"), Test::Empty, Some("x")),
            "right",
            DomainError::NotExpected,
        ),
        (
            rule(var("A"), Test::NumberGt, Some("ten")),
            "right.literal",
            number.clone(),
        ),
        (
            rule(var("A"), Test::NumberLe, Some("inf")),
            "right.literal",
            number,
        ),
        (
            rule(var("1A"), Test::NotEmpty, None),
            "left.var",
            name.clone(),
        ),
        (
            rule(large, Test::Equals, Some("x")),
            "left.literal",
            DomainError::TooLarge {
                max: Node::MAX_TEXT,
            },
        ),
        (
            rule(output("a", "bad-key"), Test::NotEmpty, None),
            "left.output.key",
            name,
        ),
    ];

    // when
    let refusals = rules.map(|(rule, field, error)| {
        let refused = refusal(vec![switch("s", &[], vec![rule])]);
        (
            refused,
            (format!("nodes[0].switch.cases[0].rules[0].{field}"), error),
        )
    });

    // then
    for (refused, expected) in refusals {
        assert_eq!(refused, expected);
    }
}

/// An output must name a run node that the switch reaches through its needs.
#[test]
fn an_output_must_come_before_the_switch() {
    // given
    let at = "nodes[2].switch.cases[0].rules[0].left.output.node";
    let reads = |node: &str| vec![rule(output(node, "KEY"), Test::NotEmpty, None)];
    let not_before = vec![
        node("a", &[]),
        node("b", &[]),
        switch("s", &["a"], reads("b")),
    ];
    let of_switch = vec![
        node("a", &[]),
        switch("t", &["a"], vec![on_var()]),
        switch("s", &["t"], reads("t")),
    ];
    let unknown = vec![
        node("a", &[]),
        node("b", &[]),
        switch("s", &["a"], reads("zz")),
    ];

    // when
    let refusals = refusals(vec![
        (not_before, at, DomainError::NotAnAncestor),
        (of_switch, at, DomainError::NotAnAncestor),
        (unknown, at, DomainError::NotAnAncestor),
    ]);

    // then
    for (refused, expected) in refusals {
        assert_eq!(refused, expected);
    }
}

/// A merge needs one need or more, and has no environment and no retries.
#[test]
fn a_bad_merge_is_refused() {
    // given
    let mut env = merge("m", &["a"]);
    env.env = vec![EnvVar {
        name: "A".to_owned(),
        value: EnvValue::Literal("1".to_owned()),
    }];
    let mut retries = merge("m", &["a"]);
    retries.retries = 2;

    // when
    let refusals = refusals(vec![
        (vec![merge("m", &[])], "nodes[0].needs", DomainError::Empty),
        (
            vec![node("a", &[]), env],
            "nodes[1].env",
            DomainError::NotExpected,
        ),
        (
            vec![node("a", &[]), retries],
            "nodes[1].retries",
            DomainError::NotExpected,
        ),
    ]);

    // then
    for (refused, expected) in refusals {
        assert_eq!(refused, expected);
    }
}
