use crate::domain::errors::{DomainError, DomainResult};

/// `TriggerRunner` is the organization's in-process identity for trigger fires: the server
/// provisions it and no user action can create, disable or delete it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppKind {
    Standard,
    TriggerRunner,
}

impl AppKind {
    pub fn new(value: impl AsRef<str>) -> DomainResult<Self> {
        match value.as_ref() {
            "standard" => Ok(Self::Standard),
            "trigger_runner" => Ok(Self::TriggerRunner),
            other => Err(DomainError::validation(format!(
                "Invalid app kind: {other}"
            ))),
        }
    }

    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::TriggerRunner => "trigger_runner",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_its_text() {
        for kind in [AppKind::Standard, AppKind::TriggerRunner] {
            assert_eq!(AppKind::new(kind.as_str()).unwrap(), kind);
        }
        assert!(AppKind::new("agent").is_err());
    }
}
