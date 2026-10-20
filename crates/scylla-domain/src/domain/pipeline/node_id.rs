use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::text::{Rule, Text};

pub enum NodeIdRule {}

impl Rule for NodeIdRule {
    const LABEL: &'static str = "Node ID";
    const MAX: usize = 128;

    fn check(s: &str) -> DomainResult<()> {
        if !s
            .chars()
            .all(|c| (c.is_ascii_alphanumeric() && !c.is_ascii_uppercase()) || c == '-' || c == '_')
        {
            return Err(DomainError::validation(
                "Node ID may only contain lowercase alphanumeric characters, hyphens, and underscores",
            ));
        }
        Ok(())
    }
}

pub type NodeId = Text<NodeIdRule>;
