use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::pipeline::{EnvKey, EnvValue};
use crate::domain::text::{Rule, Text};
use serde::{Deserialize, Serialize};

/// RFC 6901.
pub enum JsonPointerRule {}

impl Rule for JsonPointerRule {
    const LABEL: &'static str = "JSON pointer";
    const MAX: usize = 1024;

    fn sanitize(raw: String) -> String {
        raw
    }

    fn check(s: &str) -> DomainResult<()> {
        if !s.starts_with('/') {
            return Err(DomainError::validation("JSON pointer must start with '/'"));
        }
        if s.split('~')
            .skip(1)
            .any(|rest| !rest.starts_with(['0', '1']))
        {
            return Err(DomainError::validation(
                "JSON pointer must follow every '~' with '0' or '1'",
            ));
        }
        Ok(())
    }
}

pub type JsonPointer = Text<JsonPointerRule>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerInputSource {
    Literal(EnvValue),
    JsonPointer(JsonPointer),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerInput {
    key: EnvKey,
    source: TriggerInputSource,
}

impl TriggerInput {
    pub fn literal(key: EnvKey, value: impl Into<String>) -> DomainResult<Self> {
        Ok(Self {
            key,
            source: TriggerInputSource::Literal(EnvValue::new(value)?),
        })
    }

    pub fn json_pointer(key: EnvKey, pointer: impl Into<String>) -> DomainResult<Self> {
        Ok(Self {
            key,
            source: TriggerInputSource::JsonPointer(JsonPointer::new(pointer)?),
        })
    }

    #[must_use]
    pub fn key(&self) -> &EnvKey {
        &self.key
    }

    #[must_use]
    pub fn source(&self) -> &TriggerInputSource {
        &self.source
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: &str) -> EnvKey {
        EnvKey::new(k).unwrap()
    }

    #[test]
    fn json_pointer_requires_leading_slash() {
        assert!(TriggerInput::json_pointer(key("GIT_COMMIT"), "/after").is_ok());
        assert!(TriggerInput::json_pointer(key("GIT_COMMIT"), "after").is_err());
        assert!(TriggerInput::json_pointer(key("GIT_COMMIT"), "").is_err());
    }

    #[test]
    fn json_pointer_follows_rfc_6901_escapes() {
        for ok in ["/a~0b", "/a~1b/c", "/", "/a b"] {
            assert!(TriggerInput::json_pointer(key("A"), ok).is_ok(), "{ok}");
        }
        for bad in ["/a~2", "/a~", "/~x", "/a\0"] {
            assert!(TriggerInput::json_pointer(key("A"), bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn literal_rejects_nul_and_oversize() {
        assert!(TriggerInput::literal(key("A"), "x\0y").is_err());
        assert!(TriggerInput::literal(key("A"), "a".repeat(65_537)).is_err());
    }

    #[test]
    fn source_json_is_externally_tagged() {
        let lit = TriggerInput::literal(key("RUN_MODE"), "nightly").unwrap();
        let json = serde_json::to_string(&lit).unwrap();
        assert!(json.contains(r#""literal":"nightly""#), "{json}");

        let ptr = TriggerInput::json_pointer(key("GIT_REF"), "/ref").unwrap();
        let json = serde_json::to_string(&ptr).unwrap();
        assert!(json.contains(r#""json_pointer":"/ref""#), "{json}");
    }
}
