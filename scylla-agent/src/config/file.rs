//! The file of the agent, in JSON, readable by its user only: the mode 600 on Unix, and the
//! access of the parent folder on Windows.

use std::fs::{self, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::agent::AgentError;

/// What the agent keeps after its join. The private key never leaves this file.
#[derive(Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// The URL of the server, for example `https://scylla.example.com`.
    pub url: String,
    /// The id of the App.
    pub app_id: String,
    /// The id of the key of the App.
    pub key_id: String,
    /// The Ed25519 private key, in hex.
    pub private_key: String,
}

impl std::fmt::Debug for AgentConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentConfig")
            .field("url", &self.url)
            .field("app_id", &self.app_id)
            .field("key_id", &self.key_id)
            .field("private_key", &"***")
            .finish()
    }
}

impl AgentConfig {
    /// Reads the file.
    pub fn load(path: &Path) -> Result<Self, AgentError> {
        let text = fs::read_to_string(path).map_err(|error| AgentError::File(error.to_string()))?;
        serde_json::from_str(&text).map_err(|error| AgentError::File(error.to_string()))
    }

    /// Writes the file with the mode 600 on Unix, and its folder if it is missing.
    pub fn save(&self, path: &Path) -> Result<(), AgentError> {
        let fail = |error: std::io::Error| AgentError::File(error.to_string());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(fail)?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| AgentError::File(error.to_string()))?;
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(path).map_err(fail)?;
        file.write_all(text.as_bytes()).map_err(fail)
    }
}
