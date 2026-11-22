//! The ids of the nodes, the names of their ports, and the needs that join them.

use crate::DomainError;

/// The id of a node, unique in its pipeline: 1 to 128 lower case letters, digits, dashes or
/// underscores.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(String);

impl NodeId {
    /// The maximum number of characters.
    pub const MAX_LEN: usize = 128;

    /// Checks the form of an id.
    pub fn parse(raw: String) -> Result<Self, DomainError> {
        if id_form(&raw) {
            Ok(Self(raw))
        } else {
            Err(DomainError::NotANodeId)
        }
    }

    /// Returns the id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The name of a port: the form of a `NodeId`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PortName(String);

impl PortName {
    /// The port of a run node that opens when the node failed after its last attempt.
    pub const ERROR: &str = "error";

    /// Checks the form of a port name.
    pub fn parse(raw: String) -> Result<Self, DomainError> {
        if id_form(&raw) {
            Ok(Self(raw))
        } else {
            Err(DomainError::NotAPortName)
        }
    }

    /// Returns the name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Returns `true` for 1 to 128 lower case letters, digits, dashes or underscores.
fn id_form(raw: &str) -> bool {
    let allowed = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_');
    (1..=NodeId::MAX_LEN).contains(&raw.len()) && raw.chars().all(allowed)
}

/// A need of a node: another node, and a port of it or no port.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Need {
    /// The node.
    pub node: NodeId,
    /// The port. `None` opens when the node completed.
    pub port: Option<PortName>,
}
