use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::text::{Rule, Text};

pub enum UsernameRule {}

/// Login reads an identifier with '@' as an email.
impl Rule for UsernameRule {
    const LABEL: &'static str = "Username";
    const MAX: usize = 255;

    fn check(s: &str) -> DomainResult<()> {
        if s.contains('@') {
            return Err(DomainError::validation("Username cannot contain '@'"));
        }
        Ok(())
    }
}

pub type Username = Text<UsernameRule>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_and_trimming() {
        assert!(Username::new("valid_user").is_ok());
        let trimmed = Username::new("  myuser  ").unwrap();
        assert_eq!(trimmed.as_str(), "myuser");
        assert!(Username::new("").is_err());
        assert!(Username::new("   ").is_err());
    }

    #[test]
    fn length_bounds() {
        assert!(Username::new("A").is_ok());
        assert!(Username::new("a".repeat(UsernameRule::MAX)).is_ok());
        assert!(Username::new("a".repeat(UsernameRule::MAX + 1)).is_err());
    }

    #[test]
    fn rejects_an_at_sign() {
        assert!(Username::new("alice").is_ok());
        assert_eq!(
            Username::new("a@b.com").unwrap_err().to_string(),
            "Validation failed: Username cannot contain '@'"
        );
    }
}
