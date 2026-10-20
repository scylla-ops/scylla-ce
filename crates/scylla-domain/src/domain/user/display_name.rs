use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::text::{Rule, Text};

pub const DISPLAY_NAME_MAX_CHARS: usize = 100;

pub enum DisplayNameRule {}

/// The limit counts characters, not bytes: `MAX` only bounds the bytes of 100 characters.
impl Rule for DisplayNameRule {
    const LABEL: &'static str = "Display name";
    const MAX: usize = DISPLAY_NAME_MAX_CHARS * 4;

    fn check(s: &str) -> DomainResult<()> {
        if s.chars().count() > DISPLAY_NAME_MAX_CHARS {
            return Err(DomainError::validation(format!(
                "Display name cannot exceed {DISPLAY_NAME_MAX_CHARS} characters"
            )));
        }
        Ok(())
    }
}

pub type DisplayName = Text<DisplayNameRule>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_and_refuses_blank() {
        assert_eq!(DisplayName::new("  Ada  ").unwrap().as_str(), "Ada");
        assert!(DisplayName::new("").is_err());
        assert!(DisplayName::new("   ").is_err());
    }

    #[test]
    fn the_limit_counts_characters_not_bytes() {
        assert!(DisplayName::new("é".repeat(DISPLAY_NAME_MAX_CHARS)).is_ok());
        assert!(DisplayName::new("a".repeat(DISPLAY_NAME_MAX_CHARS)).is_ok());
        assert_eq!(
            DisplayName::new("é".repeat(DISPLAY_NAME_MAX_CHARS + 1))
                .unwrap_err()
                .to_string(),
            "Validation failed: Display name cannot exceed 100 characters"
        );
        assert!(DisplayName::new("😀".repeat(DISPLAY_NAME_MAX_CHARS)).is_ok());
    }

    #[test]
    fn refuses_a_control_character() {
        for bad in ["a\nb", "a\tb", "a\u{7f}b", "a\0b"] {
            assert!(DisplayName::new(bad).is_err(), "{bad:?}");
        }
    }
}
