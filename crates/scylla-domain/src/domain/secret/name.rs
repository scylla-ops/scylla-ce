use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::text::{Rule, Text};

pub enum SecretNameRule {}

impl Rule for SecretNameRule {
    const LABEL: &'static str = "Secret name";
    const MAX: usize = 128;

    fn check(s: &str) -> DomainResult<()> {
        if !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            return Err(DomainError::validation(
                "Secret name may only contain alphanumeric characters, '-', '_', and '.'",
            ));
        }
        Ok(())
    }
}

pub type SecretName = Text<SecretNameRule>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_names() {
        for n in ["DB_PASSWORD", "api.token", "prod-key", "x"] {
            assert!(SecretName::new(n).is_ok(), "{n} should be valid");
        }
    }

    #[test]
    fn rejects_bad_names() {
        for n in ["", "with space", "weird$char", "a/b"] {
            assert!(SecretName::new(n).is_err(), "{n} should be rejected");
        }
    }
}
