//! The blocks of a pipeline that link a version of a fragment of the library, and their checks
//! against the nodes of the pipeline.

use std::collections::{BTreeMap, HashSet};

use super::needs::NodeId;
use super::nodes::{env_name, Node, NodeError};
use crate::tenancy::library::FragmentId;
use crate::DomainError;

/// A block of a pipeline that holds nodes of the pipeline and links a version of a fragment. The
/// fragment can be gone: the block keeps its id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FragmentLink {
    /// The id of the block in the graph. No node has it.
    pub block_id: NodeId,
    /// The fragment.
    pub fragment_id: FragmentId,
    /// The version of the fragment that the block holds.
    pub version: u32,
    /// The values of the parameters, by key.
    pub values: BTreeMap<String, String>,
    /// The nodes of the pipeline that the block holds.
    pub node_ids: Vec<NodeId>,
    /// The newest version that the pipeline chose not to take.
    pub skipped_version: Option<u32>,
}

impl FragmentLink {
    /// The most links of a pipeline: one for each node at most.
    pub const MAX: usize = 256;
}

/// Checks the links of a pipeline against its nodes: unique blocks that no node has, versions of
/// 1 or more, short values of variable names, and nodes of the pipeline in one block only.
pub fn check_links(links: &[FragmentLink], nodes: &[Node]) -> Result<(), NodeError> {
    if links.len() > FragmentLink::MAX {
        let error = DomainError::TooMany {
            max: FragmentLink::MAX,
        };
        return Err(NodeError::at("fragment_links", error));
    }
    let ids: HashSet<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
    let mut blocks = HashSet::new();
    let mut held = HashSet::new();
    for (index, link) in links.iter().enumerate() {
        let at =
            |field: &str, error| NodeError::at(format!("fragment_links[{index}].{field}"), error);

        // Check the block, the versions and the values.
        let block = link.block_id.as_str();
        if ids.contains(block) || !blocks.insert(block) {
            return Err(at("block_id", DomainError::Duplicate));
        }
        if link.version == 0 || link.skipped_version == Some(0) {
            return Err(at(
                "version",
                DomainError::OutOfRange {
                    min: 1,
                    max: u32::MAX.into(),
                },
            ));
        }
        if link.values.len() > Node::MAX_ENV {
            return Err(at("values", DomainError::TooMany { max: Node::MAX_ENV }));
        }
        for (key, value) in &link.values {
            env_name(key).map_err(|error| at("values", error))?;
            if value.len() > Node::MAX_TEXT {
                return Err(at(
                    "values",
                    DomainError::TooLarge {
                        max: Node::MAX_TEXT,
                    },
                ));
            }
        }

        // Check the nodes: 1 or more, each a node of the pipeline in no other block.
        if link.node_ids.is_empty() {
            return Err(at("node_ids", DomainError::Empty));
        }
        for (position, node) in link.node_ids.iter().enumerate() {
            if !ids.contains(node.as_str()) || !held.insert(node.as_str()) {
                let path = format!("fragment_links[{index}].node_ids[{position}]");
                return Err(NodeError::at(path, DomainError::NotAFreeNode));
            }
        }
    }
    Ok(())
}
