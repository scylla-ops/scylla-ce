use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::secret::SecretName;
use crate::domain::text::{Rule, Text};
use serde::{Deserialize, Serialize};

pub const MAX_ENV_VARS: usize = 128;

/// The agent injects its context vars under this prefix; user keys may not shadow them.
const RESERVED_PREFIX: &str = "SCYLLA_";

pub enum EnvKeyRule {}

impl Rule for EnvKeyRule {
    const LABEL: &'static str = "Env var key";
    const MAX: usize = 255;

    fn check(s: &str) -> DomainResult<()> {
        if !s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
            return Err(DomainError::validation(
                "Env var key must start with a letter or underscore",
            ));
        }
        if !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(DomainError::validation(
                "Env var key may only contain letters, digits, and underscores",
            ));
        }
        if s.starts_with(RESERVED_PREFIX) {
            return Err(DomainError::validation(format!(
                "Env var key may not start with the reserved prefix `{RESERVED_PREFIX}`"
            )));
        }
        Ok(())
    }
}

pub type EnvKey = Text<EnvKeyRule>;

/// 64 KiB keeps one `KEY=value` string below the Linux `MAX_ARG_STRLEN` (128 KiB).
pub enum EnvValueRule {}

impl Rule for EnvValueRule {
    const LABEL: &'static str = "Env var value";
    const MAX: usize = 65_536;
    const REQUIRED: bool = false;
    const FREE_FORM: bool = true;

    fn sanitize(raw: String) -> String {
        raw
    }
}

pub type EnvValue = Text<EnvValueRule>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvSource {
    Literal(EnvValue),
    Secret(SecretName),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvVar {
    key: EnvKey,
    source: EnvSource,
}

impl EnvVar {
    pub fn literal(key: EnvKey, value: impl Into<String>) -> DomainResult<Self> {
        Ok(Self {
            key,
            source: EnvSource::Literal(EnvValue::new(value)?),
        })
    }

    #[must_use]
    pub fn secret(key: EnvKey, secret: SecretName) -> Self {
        Self {
            key,
            source: EnvSource::Secret(secret),
        }
    }

    #[must_use]
    pub fn key(&self) -> &str {
        self.key.as_str()
    }

    #[must_use]
    pub fn source(&self) -> &EnvSource {
        &self.source
    }

    #[must_use]
    pub fn literal_value(&self) -> Option<&str> {
        match &self.source {
            EnvSource::Literal(v) => Some(v.as_str()),
            EnvSource::Secret(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_keys() {
        for k in ["PATH", "MY_VAR", "_x", "a1_b2"] {
            assert!(EnvKey::new(k).is_ok(), "{k} should be valid");
        }
    }

    #[test]
    fn rejects_bad_keys() {
        for k in ["", "1ABC", "with-dash", "with space", "SCYLLA_JOB_ID"] {
            assert!(EnvKey::new(k).is_err(), "{k} should be rejected");
        }
    }

    #[test]
    fn literal_keeps_the_value_as_is_but_rejects_nul_and_oversize() {
        let key = || EnvKey::new("K").unwrap();
        let var = EnvVar::literal(key(), " a\nb ").unwrap();
        assert_eq!(var.literal_value(), Some(" a\nb "));
        assert!(EnvVar::literal(key(), "").is_ok());
        assert!(EnvVar::literal(key(), "x\0y").is_err());
        assert!(EnvVar::literal(key(), "a".repeat(EnvValueRule::MAX)).is_ok());
        assert!(EnvVar::literal(key(), "a".repeat(EnvValueRule::MAX + 1)).is_err());
    }
}
