use crate::domain::ids::new_id;
use std::fmt;

/// One id per `Actions::run`, so a journal can group the stages of one action across hooks.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActionId(String);

impl ActionId {
    pub(super) fn generate() -> Self {
        Self(new_id())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ActionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
