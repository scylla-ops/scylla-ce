//! The graph of the nodes of a pipeline: 1 to 256 nodes, unique ids, needs that name other
//! nodes, and no cycle.

use std::collections::{HashMap, HashSet};

use super::nodes::{Node, NodeError, Runtime};
use crate::DomainError;

/// The checked nodes of a pipeline, in the order of the request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nodes(Vec<Node>);

impl Nodes {
    /// The most nodes in a pipeline.
    pub const MAX: usize = 256;

    /// Checks each node, then the graph: unique ids, needs that name other nodes once, and no
    /// cycle.
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

        // Refuse a need of an unknown node, of the node itself, or a need two times.
        for (index, node) in nodes.iter().enumerate() {
            let mut needs = HashSet::new();
            for (position, need) in node.needs.iter().enumerate() {
                let path = format!("nodes[{index}].needs[{position}]");
                if need == &node.id || !ids.contains(need.as_str()) {
                    return Err(NodeError::at(path, DomainError::UnknownNode));
                }
                if !needs.insert(need.as_str()) {
                    return Err(NodeError::at(path, DomainError::Duplicate));
                }
            }
        }

        // Refuse a cycle: each node must leave the graph in the order of its needs.
        if !drains(&nodes) {
            return Err(NodeError::at("nodes", DomainError::Cycle));
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
        for node in nodes
            .iter()
            .filter(|node| node.needs.iter().any(|need| need.as_str() == id))
        {
            let count = waiting.get_mut(node.id.as_str()).expect("a known node");
            *count -= 1;
            if *count == 0 {
                ready.push(node.id.as_str());
            }
        }
    }
    done == nodes.len()
}
