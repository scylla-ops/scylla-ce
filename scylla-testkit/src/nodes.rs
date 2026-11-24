//! The nodes of a pipeline for a test: a run node, a switch with its cases and rules, and a merge.
//! A need is `node`, or `node:port` for a port.

use scylla_proto::pipelines::v1::{
    node, operand, Case, Combinator, Exec, Merge, Need, Node, NodeOutput, Operand, Rule, Switch,
    Test,
};

/// Reads a need: `node`, or `node:port`.
fn need(text: &str) -> Need {
    let (node, port) = text.split_once(':').unwrap_or((text, ""));
    Need {
        node: node.to_owned(),
        port: port.to_owned(),
    }
}

/// Makes a node that runs a program, after its needs.
pub fn exec_node(node_id: &str, needs: &[&str]) -> Node {
    Node {
        id: node_id.to_owned(),
        needs: needs.iter().map(|text| need(text)).collect(),
        step: Some(node::Step::Exec(Exec {
            argv: vec!["make".to_owned(), node_id.to_owned()],
        })),
        env: Vec::new(),
        timeout_seconds: 0,
        retries: 0,
        working_dir: String::new(),
        container: None,
    }
}

/// Makes a switch node of cases and an otherwise port (empty for none), after its needs.
pub fn switch_node(node_id: &str, needs: &[&str], cases: Vec<Case>, otherwise: &str) -> Node {
    let mut node = exec_node(node_id, needs);
    node.step = Some(node::Step::Switch(Switch {
        cases,
        otherwise: otherwise.to_owned(),
        all_matches: false,
    }));
    node
}

/// Makes a merge node after its needs.
pub fn merge_node(node_id: &str, needs: &[&str]) -> Node {
    let mut node = exec_node(node_id, needs);
    node.step = Some(node::Step::Merge(Merge {}));
    node
}

/// Makes a case of a port whose rules must each match.
pub fn case(port: &str, rules: Vec<Rule>) -> Case {
    Case {
        port: port.to_owned(),
        combinator: Combinator::All.into(),
        rules,
    }
}

/// Makes a rule with a right value, if any.
pub fn rule(left: Operand, test: Test, right: Option<Operand>) -> Rule {
    Rule {
        left: Some(left),
        test: test.into(),
        right,
    }
}

/// Returns an operand of a text.
pub fn literal(text: &str) -> Operand {
    Operand {
        value: Some(operand::Value::Literal(text.to_owned())),
    }
}

/// Returns an operand of a variable.
pub fn var(name: &str) -> Operand {
    Operand {
        value: Some(operand::Value::Var(name.to_owned())),
    }
}

/// Returns an operand of a key of the output of a node.
pub fn output(node_id: &str, key: &str) -> Operand {
    let output = NodeOutput {
        node: node_id.to_owned(),
        key: key.to_owned(),
    };
    Operand {
        value: Some(operand::Value::Output(output)),
    }
}
