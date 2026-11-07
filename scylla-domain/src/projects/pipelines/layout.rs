//! The places of the nodes on the canvas of a pipeline.

use std::collections::BTreeMap;

use super::NodeId;
use crate::DomainError;

/// A place on the canvas, in whole pixels of the graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    /// The horizontal place.
    pub x: i32,
    /// The vertical place.
    pub y: i32,
}

impl Position {
    /// The largest distance from the origin on one axis.
    pub const MAX: i32 = 1_000_000;

    /// Checks a place.
    pub fn parse(x: i32, y: i32) -> Result<Self, DomainError> {
        let range = -Self::MAX..=Self::MAX;
        if range.contains(&x) && range.contains(&y) {
            Ok(Self { x, y })
        } else {
            Err(DomainError::OffCanvas { max: Self::MAX })
        }
    }
}

/// The places of a pipeline. A node with no place gets an automatic place in the client.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PipelineLayout {
    /// The place of the start node of the canvas. It is not a node of the pipeline.
    pub start: Option<Position>,
    /// The place of each node.
    pub nodes: BTreeMap<NodeId, Position>,
}

impl PipelineLayout {
    /// The largest number of places of nodes in a request.
    pub const MAX_NODES: usize = 256;

    /// Returns `true` if the layout holds no place.
    pub fn is_empty(&self) -> bool {
        self.start.is_none() && self.nodes.is_empty()
    }
}
