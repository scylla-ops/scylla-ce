//! The file of the agent, in JSON, readable by its user only: the mode 600 on Unix, and the
//! access of the parent folder on Windows.

use std::fmt::Display;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
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
        let text = fs::read_to_string(path).map_err(|error| match error.kind() {
            ErrorKind::NotFound => AgentError::File(format!(
                "{} does not exist. Run `scylla-agent join <url> <code>`, or set SCYLLA_URL and \
                 SCYLLA_JOIN_CODE.",
                path.display()
            )),
            _ => failure(path, error),
        })?;
        serde_json::from_str(&text).map_err(|error| failure(path, error))
    }

    /// Checks that the file can be written, and makes its folder if it is missing.
    pub fn check_writable(path: &Path) -> Result<(), AgentError> {
        let fail = |error: std::io::Error| failure(path, error);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(fail)?;
        }
        if path.exists() {
            let opened = OpenOptions::new().write(true).open(path);
            return opened.map(drop).map_err(fail);
        }
        let probe = path.with_extension("check");
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&probe)
            .map_err(fail)?;
        fs::remove_file(&probe).map_err(fail)
    }

    /// Writes the file with the mode 600 on Unix, and its folder if it is missing.
    pub fn save(&self, path: &Path) -> Result<(), AgentError> {
        let fail = |error: std::io::Error| failure(path, error);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(fail)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|error| failure(path, error))?;
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(path).map_err(fail)?;
        file.write_all(text.as_bytes()).map_err(fail)
    }
}

/// An error of the file, with its path.
fn failure(path: &Path, error: impl Display) -> AgentError {
    AgentError::File(format!("{}: {error}", path.display()))
}
