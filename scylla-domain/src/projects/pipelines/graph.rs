//! The graph of the nodes of a pipeline: 1 to 256 nodes, unique ids, needs that name other
//! nodes and their ports, no cycle, and outputs of the nodes before each switch.

use std::collections::{HashMap, HashSet};

use super::needs::{Need, PortName};
use super::nodes::{Node, NodeError, Runtime, Step};
use crate::DomainError;

/// The checked nodes of a pipeline, in the order of the request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nodes(Vec<Node>);

impl Nodes {
    /// The most nodes in a pipeline.
    pub const MAX: usize = 256;

    /// Checks each node, then the graph: unique ids, needs that name other nodes and their ports
    /// once, no cycle, and outputs of the run nodes before each switch.
    pub fn parse(nodes: Vec<Node>) -> Result<Self, NodeError> {
        // Check the count, then each node.
        if nodes.is_empty() {
            return Err(NodeError::at("nodes", DomainError::Empty));
        }
        if nodes.len() > Self::MAX {
            return Err(NodeError::at(
                "nodes",
                DomainError::TooMany { max: Self::MAX },
            ));
        }
        let nodes = nodes
            .into_iter()
            .enumerate()
            .map(|(index, node)| node.check(&format!("nodes[{index}]")))
            .collect::<Result<Vec<_>, _>>()?;

        // Refuse a second node with the same id.
        let mut ids = HashSet::new();
        for (index, node) in nodes.iter().enumerate() {
            if !ids.insert(node.id.as_str()) {
                return Err(NodeError::at(
                    format!("nodes[{index}].id"),
                    DomainError::Duplicate,
                ));
            }
        }

        // Refuse a need of the node itself, of an unknown node or port, or a need two times.
        let steps: HashMap<&str, &Step> = nodes.iter().map(|n| (n.id.as_str(), &n.step)).collect();
        for (index, node) in nodes.iter().enumerate() {
            let mut needs = HashSet::new();
            for (position, need) in node.needs.iter().enumerate() {
                let path = format!("nodes[{index}].needs[{position}]");
                let Some(step) = steps
                    .get(need.node.as_str())
                    .filter(|_| need.node != node.id)
                else {
                    return Err(NodeError::at(
                        format!("{path}.node"),
                        DomainError::UnknownNode,
                    ));
                };
                if !port_of(step, need) {
                    return Err(NodeError::at(
                        format!("{path}.port"),
                        DomainError::UnknownPort,
                    ));
                }
                if !needs.insert(need) {
                    return Err(NodeError::at(path, DomainError::Duplicate));
                }
            }
        }

        // Refuse a cycle: each node must leave the graph in the order of its needs.
        if !drains(&nodes) {
            return Err(NodeError::at("nodes", DomainError::Cycle));
        }

        // Refuse an output of a node that is not a run node before its switch.
        for (index, node) in nodes.iter().enumerate() {
            let Step::Switch(switch) = &node.step else {
                continue;
            };
            let before = ancestors(&nodes, node);
            let at = format!("nodes[{index}].switch");
            for (path, output) in switch.outputs(&at) {
                let runs = steps.get(output.as_str()).is_some_and(|step| step.runs());
                if !runs || !before.contains(output.as_str()) {
                    return Err(NodeError::at(path, DomainError::NotAnAncestor));
                }
            }
        }
        Ok(Self(nodes))
    }

    /// Returns the nodes.
    pub fn as_slice(&self) -> &[Node] {
        &self.0
    }

    /// Returns the nodes.
    pub fn into_vec(self) -> Vec<Node> {
        self.0
    }

    /// Returns `true` if a node runs in a container.
    pub fn need_container(&self) -> bool {
        self.0
            .iter()
            .any(|node| matches!(node.runtime, Runtime::Container { .. }))
    }
}

/// Returns `true` if the port of a need fits its node: no port, a port of a switch, or the port
/// `error` of a run node. A merge has no port.
fn port_of(step: &Step, need: &Need) -> bool {
    match (&need.port, step) {
        (None, _) => true,
        (Some(port), Step::Switch(switch)) => switch.has_port(port),
        (Some(port), step) => step.runs() && port.as_str() == PortName::ERROR,
    }
}

/// Returns the ids of each node that a node reaches through its needs, at any depth.
fn ancestors<'a>(nodes: &'a [Node], node: &'a Node) -> HashSet<&'a str> {
    let mut seen = HashSet::new();
    let mut next: Vec<&Node> = vec![node];
    while let Some(current) = next.pop() {
        for need in &current.needs {
            if seen.insert(need.node.as_str()) {
                next.extend(nodes.iter().filter(|other| other.id == need.node));
            }
        }
    }
    seen
}

/// Returns `true` if the nodes have no cycle: the order of Kahn takes each node.
fn drains(nodes: &[Node]) -> bool {
    let mut waiting: HashMap<&str, usize> = nodes
        .iter()
        .map(|node| (node.id.as_str(), node.needs.len()))
        .collect();
    let mut ready: Vec<&str> = waiting
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut done = 0;
    while let Some(id) = ready.pop() {
        done += 1;
        for node in nodes {
            let hits = node.needs.iter().filter(|need| need.node.as_str() == id);
            let hits = hits.count();
            if hits == 0 {
                continue;
            }
            let count = waiting.get_mut(node.id.as_str()).expect("a known node");
            *count -= hits;
            if *count == 0 {
                ready.push(node.id.as_str());
            }
        }
    }
    done == nodes.len()
}
