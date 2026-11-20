//! The pipeline files of a repository: `.scylla/pipeline.<ext>` or `.scylla/pipelines/<key>.<ext>`,
//! in TOML, YAML or JSON. A file gives the same nodes as the editor, through the checks of the
//! nodes.

mod checkout;
mod convert;
mod document;
mod lookup;
mod positions;
mod read;
#[cfg(test)]
mod tests;
mod warnings;
mod write;

use std::fmt;

use serde::{Deserialize, Serialize};

pub use checkout::CheckoutSettings;
pub use document::{
    json_schema, ContainerFile, EnvEntry, EnvTable, NodeFile, PipelineFile, ShellName,
};
pub use lookup::{lookup, FileRef, Lookup, Rejected, TreeEntry, MAX_FILES};
pub use read::parse;
pub use warnings::missing_secrets;

use crate::projects::git_rules::GitRules;
use crate::projects::pipelines::{Nodes, PipelineName};
use crate::DomainError;

/// The largest pipeline file, in bytes.
pub const MAX_BYTES: usize = 1 << 20;

/// The only version of the format.
pub const VERSION: u32 = 1;

/// The format of a pipeline file, from the extension of its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileFormat {
    /// TOML, `.toml`.
    Toml,
    /// YAML, `.yaml` or `.yml`.
    Yaml,
    /// JSON, `.json`.
    Json,
}

impl FileFormat {
    /// The extensions of a pipeline file, in the order of the lookup.
    pub const EXTENSIONS: [&'static str; 4] = ["toml", "yaml", "yml", "json"];

    /// Returns the format of an extension, or `None` for another extension.
    pub fn of_extension(extension: &str) -> Option<Self> {
        match extension {
            "toml" => Some(Self::Toml),
            "yaml" | "yml" => Some(Self::Yaml),
            "json" => Some(Self::Json),
            _ => None,
        }
    }

    /// Returns the extension of an exported file.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Toml => "toml",
            Self::Yaml => "yaml",
            Self::Json => "json",
        }
    }
}

/// The key of a pipeline file: the name of the file with no extension, or `pipeline` for the
/// file `.scylla/pipeline.<ext>`. 1 to 64 lower case letters, digits, dashes or underscores.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileKey(String);

impl FileKey {
    /// The most characters.
    pub const MAX_LEN: usize = 64;

    /// Checks the form of a key.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let allowed =
            |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_');
        if (1..=Self::MAX_LEN).contains(&raw.len()) && raw.chars().all(allowed) {
            Ok(Self(raw.to_owned()))
        } else {
            Err(DomainError::NotAFileKey)
        }
    }

    /// Returns the key as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A place in the text of a file: a line and a column, both from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    /// The line, from 1.
    pub line: u32,
    /// The column, in characters, from 1.
    pub column: u32,
}

/// An error of a pipeline file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileError {
    /// The path of the file in the repository, for example `.scylla/pipelines/build.toml`.
    pub path: String,
    /// The place of the error in the file. `None` for an error of the whole file.
    pub location: Option<Location>,
    /// The path of the value, for example `nodes[2].needs[0]`. Empty for an error of the syntax
    /// or of the whole file.
    pub field: String,
    /// What is wrong.
    pub message: String,
}

impl FileError {
    /// Makes an error of the whole file.
    pub fn of_file(path: &str, message: impl Into<String>) -> Self {
        Self {
            path: path.to_owned(),
            location: None,
            field: String::new(),
            message: message.into(),
        }
    }
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path)?;
        if let Some(location) = self.location {
            write!(f, ":{}:{}", location.line, location.column)?;
        }
        if self.field.is_empty() {
            write!(f, ": {}", self.message)
        } else {
            write!(f, ": {} {}", self.field, self.message)
        }
    }
}

/// A checked pipeline file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilePipeline {
    /// The display name: `name`, or the key of the file.
    pub name: PipelineName,
    /// The Git events that start a run. No rule: only a manual run starts it.
    pub rules: GitRules,
    /// The checkout of the commit before the first node.
    pub checkout: CheckoutSettings,
    /// The checked nodes.
    pub nodes: Nodes,
}

impl FilePipeline {
    /// Returns the text of the pipeline in a format. Each format reads back to the same
    /// pipeline.
    pub fn export(&self, format: FileFormat) -> String {
        PipelineFile::of(
            self.name.as_str(),
            self.nodes.as_slice(),
            &self.rules,
            &self.checkout,
        )
        .write(format)
    }
}
