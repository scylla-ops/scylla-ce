//! The search text of a list: a part of a name or of another text that the list keeps.

use super::DomainError;

/// The search text of a list: trimmed, at most `MAX_LEN` characters, kept in lower case. An empty
/// text keeps each item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchText(String);

impl SearchText {
    /// The maximum number of characters, after trim.
    pub const MAX_LEN: usize = 100;

    /// Trims the text and checks its length.
    pub fn parse(raw: String) -> Result<Self, DomainError> {
        let trimmed = raw.trim();
        if trimmed.chars().count() > Self::MAX_LEN {
            return Err(DomainError::TooLong { max: Self::MAX_LEN });
        }
        Ok(Self(trimmed.chars().flat_map(char::to_lowercase).collect()))
    }

    /// Returns the text, in lower case.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns `true` when the text keeps each item.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A search text is trimmed and kept in lower case.
    #[test]
    fn parse_trims_and_lowers_the_text() {
        let text = SearchText::parse("  Deploy API ".to_owned()).unwrap();
        assert_eq!(text.as_str(), "deploy api");
    }

    /// A search text of more than `MAX_LEN` characters is refused.
    #[test]
    fn parse_refuses_a_long_text() {
        let long = "a".repeat(SearchText::MAX_LEN + 1);
        assert!(SearchText::parse(long).is_err());
        let longest = "é".repeat(SearchText::MAX_LEN);
        assert!(SearchText::parse(longest).is_ok());
    }
}
