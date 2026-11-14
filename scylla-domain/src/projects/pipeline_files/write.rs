//! The text of a pipeline document in each format. In TOML, each value of a node is inline, so
//! the variables keep their order.

use serde::Serialize;
use toml::ser::ValueSerializer;

use super::document::{NodeFile, PipelineFile};
use super::{CheckoutSettings, FileFormat};
use crate::projects::git_rules::GitRules;

/// The keys of a pipeline document before the nodes, for the TOML text.
#[derive(Serialize)]
struct Head {
    /// The version of the format.
    version: u32,
    /// The display name.
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// The rules of the Git events.
    #[serde(rename = "on", skip_serializing_if = "GitRules::is_empty")]
    on: GitRules,
    /// The checkout settings.
    #[serde(skip_serializing_if = "CheckoutSettings::is_default")]
    checkout: CheckoutSettings,
}

impl PipelineFile {
    /// Returns the text of the document in a format.
    pub fn write(&self, format: FileFormat) -> String {
        let text = match format {
            FileFormat::Toml => toml_text(self).map_err(|e| e.to_string()),
            FileFormat::Yaml => serde_saphyr::to_string(self).map_err(|e| e.to_string()),
            FileFormat::Json => serde_json::to_string_pretty(self).map_err(|e| e.to_string()),
        };
        text.expect("a pipeline document has only texts, numbers, lists and tables")
    }
}

/// Returns the TOML text of a document: the head as tables, then one `[[nodes]]` table for each
/// node.
fn toml_text(file: &PipelineFile) -> Result<String, toml::ser::Error> {
    let head = Head {
        version: file.version,
        name: file.name.clone(),
        on: file.on.clone(),
        checkout: file.checkout.clone(),
    };
    let mut text = toml::to_string_pretty(&head)?;
    for node in &file.nodes {
        text.push_str("\n[[nodes]]\n");
        node_text(node, &mut text)?;
    }
    Ok(text)
}

/// Writes the keys of a node that are not a default, each with an inline value.
fn node_text(node: &NodeFile, text: &mut String) -> Result<(), toml::ser::Error> {
    entry(text, "id", &node.id)?;
    if !node.needs.is_empty() {
        entry(text, "needs", &node.needs)?;
    }
    if let Some(exec) = &node.exec {
        entry(text, "exec", exec)?;
    }
    if let Some(script) = &node.script {
        entry(text, "script", script)?;
    }
    if let Some(shell) = &node.shell {
        entry(text, "shell", shell)?;
    }
    if !node.env.is_empty() {
        entry(text, "env", &node.env)?;
    }
    if let Some(seconds) = &node.timeout_seconds {
        entry(text, "timeout_seconds", seconds)?;
    }
    if let Some(retries) = &node.retries {
        entry(text, "retries", retries)?;
    }
    if let Some(dir) = &node.working_dir {
        entry(text, "working_dir", dir)?;
    }
    if let Some(container) = &node.container {
        entry(text, "container", container)?;
    }
    Ok(())
}

/// Writes one line `key = value`, with the value inline.
fn entry<T: Serialize + ?Sized>(
    text: &mut String,
    key: &str,
    value: &T,
) -> Result<(), toml::ser::Error> {
    text.push_str(key);
    text.push_str(" = ");
    value.serialize(ValueSerializer::new(text))?;
    text.push('\n');
    Ok(())
}
