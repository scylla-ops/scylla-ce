//! The nodes with no process. A switch: the read of the output files, the evaluation of its
//! cases, and its events, with a pure parse and a pure evaluation. A merge: its events.

use std::collections::HashMap;
use std::path::Path;

use scylla_proto::agents::v1::{DispatchNode, EventKind};
use scylla_proto::pipelines::v1::{operand, Case, Combinator, Operand, Rule, Switch, Test};
use tokio::io::AsyncReadExt;

use super::command;
use super::events::{self, Event};
use super::output;
use super::step::NodeContext;

/// The largest output file, in bytes.
const MAX_OUTPUT: usize = 64 << 10;

/// The keys of the output file of each node.
type Outputs = HashMap<String, HashMap<String, String>>;

/// Evaluates a switch at once, with no process, and sends its events. Returns the ports that it
/// opens, or the reason of its failure with the values of the secrets masked.
pub(super) async fn run(
    context: &NodeContext,
    node: &DispatchNode,
    switch: &Switch,
) -> Result<Vec<String>, String> {
    // Tell the start, and collect the variables of the node.
    let (job, outbox) = (&context.job, &context.outbox);
    let started = Event {
        node: &node.id,
        attempt: 1,
        ..Event::default()
    };
    events::send(outbox, job, EventKind::NodeStarted, started).await;
    let mut vars: HashMap<String, String> = command::base(context, &node.id)
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect();
    vars.extend(
        node.env
            .iter()
            .map(|var| (var.name.clone(), var.value.clone())),
    );

    // Evaluate the cases, and tell the end.
    match read_outputs(&context.dir, switch)
        .await
        .and_then(|outputs| evaluate(switch, &vars, &outputs))
    {
        Ok(ports) => {
            let event = Event {
                node: &node.id,
                ports: ports.clone(),
                ..Event::default()
            };
            events::send(outbox, job, EventKind::NodeCompleted, event).await;
            Ok(ports)
        }
        Err(error) => {
            let masks: Vec<String> = node
                .env
                .iter()
                .filter(|var| var.masked)
                .map(|var| var.value.clone())
                .collect();
            let error = output::mask(error, &masks);
            output::report(outbox, job, &node.id, &format!("attempt 1 failed: {error}")).await;
            let event = Event {
                node: &node.id,
                error: error.clone(),
                ..Event::default()
            };
            events::send(outbox, job, EventKind::NodeFailed, event).await;
            Err(error)
        }
    }
}

/// Sends the start and the end of a merge, at once.
pub(super) async fn merge(context: &NodeContext, node: &str) {
    let started = Event {
        node,
        attempt: 1,
        ..Event::default()
    };
    events::send(
        &context.outbox,
        &context.job,
        EventKind::NodeStarted,
        started,
    )
    .await;
    let completed = Event {
        node,
        ..Event::default()
    };
    events::send(
        &context.outbox,
        &context.job,
        EventKind::NodeCompleted,
        completed,
    )
    .await;
}

/// Reads the output file of each node that a switch reads. A missing file gives no key.
async fn read_outputs(dir: &Path, switch: &Switch) -> Result<Outputs, String> {
    let mut outputs = Outputs::new();
    for node in sources(switch) {
        let path = dir.join(".scylla").join("out").join(node);
        let file = match tokio::fs::File::open(&path).await {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("the output of {node}: {error}")),
        };
        let mut bytes = Vec::new();
        let limit = MAX_OUTPUT as u64 + 1;
        file.take(limit)
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| format!("the output of {node}: {error}"))?;
        let keys = parse(&bytes).map_err(|error| format!("the output of {node} {error}"))?;
        outputs.insert(node.to_owned(), keys);
    }
    Ok(outputs)
}

/// Returns the nodes whose output a switch reads.
fn sources(switch: &Switch) -> Vec<&str> {
    let mut nodes: Vec<&str> = switch
        .cases
        .iter()
        .flat_map(|case| &case.rules)
        .flat_map(|rule| [&rule.left, &rule.right])
        .filter_map(
            |operand| match operand.as_ref().and_then(|operand| operand.value.as_ref()) {
                Some(operand::Value::Output(output)) => Some(output.node.as_str()),
                _ => None,
            },
        )
        .collect();
    nodes.sort_unstable();
    nodes.dedup();
    nodes
}

/// Reads an output file: one `KEY=value` on each line, with the value after the first `=`. An
/// empty line is ignored, and a later key wins.
pub(super) fn parse(bytes: &[u8]) -> Result<HashMap<String, String>, String> {
    if bytes.len() > MAX_OUTPUT {
        return Err(format!("has more than {MAX_OUTPUT} bytes"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "is not UTF-8 text".to_owned())?;
    let mut keys = HashMap::new();
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let number = index + 1;
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("has no = on line {number}"));
        };
        if !is_key(key) {
            return Err(format!("has a bad key on line {number}"));
        }
        keys.insert(key.to_owned(), value.to_owned());
    }
    Ok(keys)
}

/// Returns `true` for a letter or an underscore, then letters, digits or underscores.
fn is_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && key.len() <= 256
}

/// Returns the ports that a switch opens: the port of the first case that matches, or of each
/// case that matches with `all_matches`, else the otherwise port if it has one.
pub(super) fn evaluate(
    switch: &Switch,
    vars: &HashMap<String, String>,
    outputs: &Outputs,
) -> Result<Vec<String>, String> {
    let mut ports = Vec::new();
    for case in &switch.cases {
        if matches(case, vars, outputs)? {
            ports.push(case.port.clone());
            if !switch.all_matches {
                break;
            }
        }
    }
    if ports.is_empty() && !switch.otherwise.is_empty() {
        ports.push(switch.otherwise.clone());
    }
    Ok(ports)
}

/// Returns `true` if a case matches: each rule for `All`, one rule or more for `Any`.
fn matches(case: &Case, vars: &HashMap<String, String>, outputs: &Outputs) -> Result<bool, String> {
    let results = case
        .rules
        .iter()
        .enumerate()
        .map(|(index, rule)| {
            check(rule, vars, outputs)
                .map_err(|error| format!("rule {} of the case {}: {error}", index + 1, case.port))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(match case.combinator() {
        Combinator::Any => results.contains(&true),
        Combinator::All | Combinator::Unspecified => !results.contains(&false),
    })
}

/// Returns `true` if a rule matches. A number test fails on a text that is not a finite number.
fn check(rule: &Rule, vars: &HashMap<String, String>, outputs: &Outputs) -> Result<bool, String> {
    let left = value(rule.left.as_ref(), vars, outputs);
    let right = value(rule.right.as_ref(), vars, outputs);
    let number = |text: &str| {
        text.trim()
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .ok_or_else(|| format!("{text:?} is not a number"))
    };
    Ok(match rule.test() {
        Test::Equals => left == right,
        Test::NotEquals => left != right,
        Test::Contains => left.contains(right),
        Test::NotContains => !left.contains(right),
        Test::StartsWith => left.starts_with(right),
        Test::EndsWith => left.ends_with(right),
        Test::Empty => left.is_empty(),
        Test::NotEmpty => !left.is_empty(),
        Test::Unspecified => return Err("the rule has no test".to_owned()),
        test => {
            let (left, right) = (number(left)?, number(right)?);
            match test {
                Test::NumberEq => left == right,
                Test::NumberNe => left != right,
                Test::NumberGt => left > right,
                Test::NumberGe => left >= right,
                Test::NumberLt => left < right,
                _ => left <= right,
            }
        }
    })
}

/// Returns the text of an operand. A missing variable, file or key gives an empty text.
fn value<'a>(
    operand: Option<&'a Operand>,
    vars: &'a HashMap<String, String>,
    outputs: &'a Outputs,
) -> &'a str {
    let found = match operand.and_then(|operand| operand.value.as_ref()) {
        Some(operand::Value::Literal(text)) => Some(text),
        Some(operand::Value::Var(name)) => vars.get(name),
        Some(operand::Value::Output(output)) => outputs
            .get(&output.node)
            .and_then(|keys| keys.get(&output.key)),
        None => None,
    };
    found.map_or("", String::as_str)
}

#[cfg(test)]
mod tests;
