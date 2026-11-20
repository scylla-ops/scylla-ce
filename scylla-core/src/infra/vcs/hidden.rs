//! A secret text that `Debug` never shows: a token, a key, a client secret or a webhook secret.

use std::fmt;

use serde::Deserialize;

/// A secret text. `Debug` shows `***`, and only `expose` gives the value.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Hidden(String);

impl Hidden {
    /// Keeps a secret text.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the secret text, for a header, a signature or a seal only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Hidden {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}
