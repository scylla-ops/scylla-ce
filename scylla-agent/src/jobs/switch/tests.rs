//! Unit tests of the evaluation of a switch and of the parse of an output file.

use std::collections::HashMap;

use scylla_proto::pipelines::v1::{
    operand, Case, Combinator, NodeOutput, Operand, Rule, Switch, Test,
};

use super::{evaluate, parse, Outputs, MAX_OUTPUT};

/// Returns an operand of a text.
fn literal(text: &str) -> Option<Operand> {
    Some(Operand {
        value: Some(operand::Value::Literal(text.to_owned())),
    })
}

/// Returns an operand of a variable.
fn var(name: &str) -> Option<Operand> {
    Some(Operand {
        value: Some(operand::Value::Var(name.to_owned())),
    })
}

/// Returns an operand of a key of the output of a node.
fn output(node: &str, key: &str) -> Option<Operand> {
    let output = NodeOutput {
        node: node.to_owned(),
        key: key.to_owned(),
    };
    Some(Operand {
        value: Some(operand::Value::Output(output)),
    })
}

/// Makes a rule.
fn rule(left: Option<Operand>, test: Test, right: Option<Operand>) -> Rule {
    Rule {
        left,
        test: test.into(),
        right,
    }
}

/// Makes a case of a port, a combinator and rules.
fn case(port: &str, combinator: Combinator, rules: Vec<Rule>) -> Case {
    Case {
        port: port.to_owned(),
        combinator: combinator.into(),
        rules,
    }
}

/// Makes a switch with the otherwise port `other`.
fn switch(cases: Vec<Case>, all_matches: bool) -> Switch {
    Switch {
        cases,
        otherwise: "other".to_owned(),
        all_matches,
    }
}

/// Returns the variables `BRANCH=main`, `COUNT=12` and `EMPTY=`.
fn vars() -> HashMap<String, String> {
    [("BRANCH", "main"), ("COUNT", "12"), ("EMPTY", "")]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}

/// Returns `true` if one rule matches the variables and no output.
fn one(rule: Rule) -> bool {
    let cases = vec![case("yes", Combinator::All, vec![rule])];
    evaluate(&switch(cases, false), &vars(), &Outputs::new()).unwrap() == ["yes"]
}

/// Each test of a text and of a number gives its result. A missing variable is an empty text.
#[test]
fn each_test_gives_its_result() {
    // given
    let rules = [
        (rule(var("BRANCH"), Test::Equals, literal("main")), true),
        (rule(var("BRANCH"), Test::NotEquals, literal("main")), false),
        (rule(var("BRANCH"), Test::Contains, literal("ai")), true),
        (rule(var("BRANCH"), Test::NotContains, literal("ai")), false),
        (rule(var("BRANCH"), Test::StartsWith, literal("ma")), true),
        (rule(var("BRANCH"), Test::EndsWith, literal("in")), true),
        (rule(var("EMPTY"), Test::Empty, None), true),
        (rule(var("MISSING"), Test::Empty, None), true),
        (rule(var("BRANCH"), Test::NotEmpty, None), true),
        (rule(var("COUNT"), Test::NumberEq, literal(" 12.0 ")), true),
        (rule(var("COUNT"), Test::NumberNe, literal("12")), false),
        (rule(var("COUNT"), Test::NumberGt, literal("9")), true),
        (rule(var("COUNT"), Test::NumberGe, literal("12")), true),
        (rule(var("COUNT"), Test::NumberLt, literal("1e3")), true),
        (rule(var("COUNT"), Test::NumberLe, literal("-1")), false),
    ];

    // when
    let results: Vec<_> = rules.iter().map(|(rule, _)| one(rule.clone())).collect();

    // then
    let expected: Vec<_> = rules.iter().map(|(_, expected)| *expected).collect();
    assert_eq!(results, expected);
}

/// `All` needs each rule, `Any` needs one rule. The first case wins with no `all_matches`, each
/// case that matches opens with it, and the otherwise port opens when no case matches.
#[test]
fn the_cases_choose_the_ports() {
    // given
    let yes = || rule(var("BRANCH"), Test::Equals, literal("main"));
    let no = || rule(var("BRANCH"), Test::Equals, literal("dev"));
    let cases = vec![
        case("all", Combinator::All, vec![yes(), no()]),
        case("any", Combinator::Any, vec![no(), yes()]),
        case("again", Combinator::Unspecified, vec![yes()]),
    ];
    let none = vec![case("all", Combinator::All, vec![no()])];
    let mut closed = switch(none.clone(), false);
    closed.otherwise.clear();

    // when
    let first = evaluate(&switch(cases.clone(), false), &vars(), &Outputs::new());
    let each = evaluate(&switch(cases, true), &vars(), &Outputs::new());
    let otherwise = evaluate(&switch(none, false), &vars(), &Outputs::new());
    let nothing = evaluate(&closed, &vars(), &Outputs::new());

    // then
    assert_eq!(first.unwrap(), ["any"]);
    assert_eq!(each.unwrap(), ["any", "again"]);
    assert_eq!(otherwise.unwrap(), ["other"]);
    assert!(nothing.unwrap().is_empty());
}

/// A number test fails the switch on a text that is not a finite number, with the reason.
#[test]
fn a_number_test_needs_numbers() {
    // given
    let cases = |right: &str| {
        let rules = vec![rule(var("BRANCH"), Test::NumberGt, literal(right))];
        switch(vec![case("big", Combinator::All, rules)], false)
    };

    // when
    let text = evaluate(&cases("1"), &vars(), &Outputs::new());
    let infinite = evaluate(&cases("inf"), &HashMap::new(), &Outputs::new());

    // then
    assert_eq!(
        text.unwrap_err(),
        "rule 1 of the case big: \"main\" is not a number"
    );
    assert_eq!(
        infinite.unwrap_err(),
        "rule 1 of the case big: \"\" is not a number"
    );
}

/// An output operand reads the key of the output of its node. A missing node or key is empty.
#[test]
fn an_output_gives_its_key() {
    // given
    let keys = parse(b"VERSION=1.2\n").unwrap();
    let outputs: Outputs = [("build".to_owned(), keys)].into_iter().collect();
    let rules = vec![
        rule(output("build", "VERSION"), Test::Equals, literal("1.2")),
        rule(output("build", "MISSING"), Test::Empty, None),
        rule(output("lint", "VERSION"), Test::Empty, None),
    ];
    let cases = vec![case("release", Combinator::All, rules)];

    // when
    let ports = evaluate(&switch(cases, false), &HashMap::new(), &outputs);

    // then
    assert_eq!(ports.unwrap(), ["release"]);
}

/// An output file has one `KEY=value` on each line. An empty line is ignored, the value is the
/// text after the first `=`, and a later key wins. A bad line or a large file is refused.
#[test]
fn an_output_file_is_parsed() {
    // given
    let good = b"A=1\n\nB=x=y\r\nA=2\nC=\n";
    let large = vec![b'x'; MAX_OUTPUT + 1];

    // when
    let keys = parse(good).unwrap();
    let refusals = [
        parse(b"A=1\nno equal sign\n"),
        parse(b"A=1\n\n1BAD=2\n"),
        parse(&large),
    ];

    // then
    assert_eq!(keys["A"], "2");
    assert_eq!(keys["B"], "x=y");
    assert_eq!(keys["C"], "");
    let errors: Vec<_> = refusals.into_iter().map(Result::unwrap_err).collect();
    assert_eq!(
        errors,
        [
            "has no = on line 2".to_owned(),
            "has a bad key on line 3".to_owned(),
            format!("has more than {MAX_OUTPUT} bytes"),
        ]
    );
}
