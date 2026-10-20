use crate::domain::pipeline::EnvValueRule;
use crate::domain::text::{Rule, Text};

/// Injected as an env var, so it shares the env value cap.
pub enum SecretValueRule {}

impl Rule for SecretValueRule {
    const LABEL: &'static str = "Secret value";
    const MAX: usize = EnvValueRule::MAX;
    const REQUIRED: bool = false;
    const FREE_FORM: bool = true;
    const SECRET: bool = true;

    fn sanitize(raw: String) -> String {
        raw
    }
}

pub type SecretValue = Text<SecretValueRule>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_bounded_and_never_printed() {
        let value = SecretValue::new("hunter2").unwrap();
        assert!(!format!("{value:?}").contains("hunter2"));
        assert!(SecretValue::new("a".repeat(SecretValueRule::MAX + 1)).is_err());
    }
}
