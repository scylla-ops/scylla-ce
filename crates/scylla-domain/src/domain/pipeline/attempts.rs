use crate::domain::errors::{DomainError, DomainResult};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct NodeTimeout(u32);

impl NodeTimeout {
    pub const MAX_SECONDS: u32 = 604_800;

    pub fn new(seconds: u32) -> DomainResult<Self> {
        if (1..=Self::MAX_SECONDS).contains(&seconds) {
            Ok(Self(seconds))
        } else {
            Err(DomainError::validation(format!(
                "Node timeout must be 1 to {} seconds",
                Self::MAX_SECONDS
            )))
        }
    }

    #[must_use]
    pub fn seconds(self) -> u32 {
        self.0
    }

    #[must_use]
    pub fn duration(self) -> Duration {
        Duration::from_secs(self.0.into())
    }
}

impl TryFrom<u32> for NodeTimeout {
    type Error = DomainError;

    fn try_from(seconds: u32) -> DomainResult<Self> {
        Self::new(seconds)
    }
}

impl From<NodeTimeout> for u32 {
    fn from(timeout: NodeTimeout) -> Self {
        timeout.0
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct NodeRetries(u32);

impl NodeRetries {
    pub const MAX: u32 = 10;

    pub fn new(retries: u32) -> DomainResult<Self> {
        if retries <= Self::MAX {
            Ok(Self(retries))
        } else {
            Err(DomainError::validation(format!(
                "Node retries must be 0 to {}",
                Self::MAX
            )))
        }
    }

    #[must_use]
    pub fn get(self) -> u32 {
        self.0
    }

    #[must_use]
    pub fn max_attempts(self) -> u32 {
        self.0 + 1
    }
}

impl TryFrom<u32> for NodeRetries {
    type Error = DomainError;

    fn try_from(retries: u32) -> DomainResult<Self> {
        Self::new(retries)
    }
}

impl From<NodeRetries> for u32 {
    fn from(retries: NodeRetries) -> Self {
        retries.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timeout_is_one_second_to_seven_days() {
        assert!(NodeTimeout::new(0).is_err());
        assert_eq!(NodeTimeout::new(1).unwrap().seconds(), 1);
        assert_eq!(
            NodeTimeout::new(604_800).unwrap().duration(),
            Duration::from_secs(604_800)
        );
        assert!(matches!(
            NodeTimeout::new(604_801),
            Err(DomainError::Validation(_))
        ));
    }

    #[test]
    fn retries_are_zero_to_ten() {
        assert_eq!(NodeRetries::default().max_attempts(), 1);
        assert_eq!(NodeRetries::new(10).unwrap().max_attempts(), 11);
        assert!(matches!(
            NodeRetries::new(11),
            Err(DomainError::Validation(_))
        ));
    }

    #[test]
    fn a_stored_value_out_of_range_does_not_read() {
        assert!(serde_json::from_str::<NodeTimeout>("0").is_err());
        assert!(serde_json::from_str::<NodeRetries>("11").is_err());
        assert_eq!(serde_json::from_str::<NodeRetries>("3").unwrap().get(), 3);
    }
}
