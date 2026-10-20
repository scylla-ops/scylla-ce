use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::text::{Rule, Text};
use serde::{Deserialize, Serialize};

/// An HTTP header name: an RFC 7230 token.
pub enum SignatureHeaderRule {}

impl Rule for SignatureHeaderRule {
    const LABEL: &'static str = "Signature header";
    const MAX: usize = 255;

    fn check(s: &str) -> DomainResult<()> {
        if !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c))
        {
            return Err(DomainError::validation(
                "Signature header must be an HTTP header name",
            ));
        }
        Ok(())
    }
}

pub type SignatureHeader = Text<SignatureHeaderRule>;

/// The signing secret lives in the AEAD secret store: HMAC needs the plaintext.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WebhookSpec {
    #[serde(default)]
    signature_header: Option<SignatureHeader>,
}

impl WebhookSpec {
    pub fn new(signature_header: Option<String>) -> DomainResult<Self> {
        Ok(Self {
            signature_header: signature_header.map(SignatureHeader::new).transpose()?,
        })
    }

    #[must_use]
    pub fn signature_header(&self) -> Option<&str> {
        self.signature_header.as_ref().map(SignatureHeader::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_header_is_an_http_token() {
        let spec = WebhookSpec::new(Some(" X-Hub-Signature-256 ".into())).unwrap();
        assert_eq!(spec.signature_header(), Some("X-Hub-Signature-256"));
        for bad in ["bad header", "X:Y", "é-header", "a\0", "  "] {
            assert!(WebhookSpec::new(Some(bad.into())).is_err(), "{bad}");
        }
        assert!(WebhookSpec::new(Some("a".repeat(SignatureHeaderRule::MAX + 1))).is_err());
    }
}
