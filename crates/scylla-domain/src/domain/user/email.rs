use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::text::{Rule, Text};

pub enum EmailRule {}

/// Light on purpose: it checks only the shape of an address.
impl Rule for EmailRule {
    const LABEL: &'static str = "Email";
    // RFC 5321
    const MAX: usize = 320;

    fn sanitize(raw: String) -> String {
        raw.trim().to_lowercase()
    }

    fn check(s: &str) -> DomainResult<()> {
        let Some((local, domain)) = s.split_once('@') else {
            return Err(DomainError::validation("Email must contain '@'"));
        };
        if local.is_empty()
            || domain.is_empty()
            || !domain.contains('.')
            || domain.starts_with('.')
            || domain.ends_with('.')
        {
            return Err(DomainError::validation("Email is not a valid address"));
        }
        Ok(())
    }
}

pub type Email = Text<EmailRule>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_and_normalises_case() {
        let e = Email::new("  Alice@Example.COM ").unwrap();
        assert_eq!(e.as_str(), "alice@example.com");
    }

    #[test]
    fn rejects_malformed() {
        assert_eq!(
            Email::new("  ").unwrap_err().to_string(),
            "Validation failed: Email cannot be empty"
        );
        assert!(Email::new("nope").is_err());
        assert!(Email::new("@example.com").is_err());
        assert!(Email::new("a@b").is_err());
        assert!(Email::new("a@.com").is_err());
        assert!(Email::new("a@b.").is_err());
    }
}
