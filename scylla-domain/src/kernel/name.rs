//! The display name of a resource or a person.

use crate::DomainError;

/// A display name: trimmed, 1 to `MAX_LEN` characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name(String);

impl Name {
    /// The maximum number of characters, after trim.
    pub const MAX_LEN: usize = 100;

    /// Trims the text and checks its length.
    pub fn parse(raw: String) -> Result<Self, DomainError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(DomainError::Empty);
        }
        if trimmed.chars().count() > Self::MAX_LEN {
            return Err(DomainError::TooLong { max: Self::MAX_LEN });
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// Returns the name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
