//! The needs of a node in the run of a job: when they ended, when they are open, and the errors
//! that the `error` needs give.

use std::collections::HashMap;

use scylla_proto::agents::v1::{dispatch_node, DispatchNode};
use scylla_proto::pipelines::v1::Need;

use super::State;

/// The port of a run node that opens when the node failed after its last attempt.
const ERROR: &str = "error";

/// Returns `true` if a node whose needs ended runs: a merge with one open need or more, another
/// node with each need open.
pub(super) fn runs(node: &DispatchNode, states: &HashMap<String, State>) -> bool {
    let open = node.links.iter().filter(|need| open(states, need)).count();
    match node.step {
        Some(dispatch_node::Step::Merge(_)) => open > 0,
        _ => open == node.links.len(),
    }
}

/// Returns `true` if the node of a need ended: it completed, failed, or did not run.
pub(super) fn ended(states: &HashMap<String, State>, need: &Need) -> bool {
    !matches!(
        states.get(&need.node),
        Some(State::Pending | State::Running)
    )
}

/// Returns `true` if a need is open: its node completed with no port asked, a switch opened its
/// port, or a run node failed for the port `error`.
fn open(states: &HashMap<String, State>, need: &Need) -> bool {
    match (states.get(&need.node), need.port.as_str()) {
        (Some(State::Completed(_)), "") => true,
        (Some(State::Completed(ports)), port) => ports.iter().any(|open| open == port),
        (Some(State::Failed(_)), ERROR) => true,
        _ => false,
    }
}

/// Returns one line for each open `error` need of a node, in the order of its needs:
/// `<node>: <error>`. `None` if no `error` need is open.
pub(super) fn errors(node: &DispatchNode, states: &HashMap<String, State>) -> Option<String> {
    let lines: Vec<String> = node
        .links
        .iter()
        .filter(|need| need.port == ERROR)
        .filter_map(|need| match states.get(&need.node) {
            Some(State::Failed(error)) => Some(format!("{}: {error}", need.node)),
            _ => None,
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Returns `true` if another node takes the failure of a node with an `error` need.
pub(super) fn routed(nodes: &[DispatchNode], id: &str) -> bool {
    nodes
        .iter()
        .flat_map(|node| &node.links)
        .any(|need| need.node == id && need.port == ERROR)
}
