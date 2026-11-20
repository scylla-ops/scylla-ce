//! The document of a pipeline file: one serde shape for TOML, YAML and JSON, and its JSON
//! Schema.

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;

use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::CheckoutSettings;
use crate::projects::git_rules::GitRules;

/// A Scylla pipeline file, version 1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Scylla pipeline file")]
pub struct PipelineFile {
    /// The version of the format. Only `1` is supported.
    pub version: u32,
    /// The display name of the pipeline: 1 to 100 characters. Default: the key of the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The Git events that start a run. With no rule, only a manual run starts the pipeline.
    #[serde(default, rename = "on", skip_serializing_if = "GitRules::is_empty")]
    pub on: GitRules,
    /// How the agent fetches the commit before the first node.
    #[serde(default, skip_serializing_if = "CheckoutSettings::is_default")]
    pub checkout: CheckoutSettings,
    /// The nodes: 1 to 256, with unique ids.
    pub nodes: Vec<NodeFile>,
}

/// A node of a pipeline file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeFile {
    /// The id of the node: 1 to 128 lower case letters, digits, dashes or underscores.
    pub id: String,
    /// The ids of the nodes that must complete before this node starts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
    /// A program and its arguments, with no shell. Set `exec` or `script`, not both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec: Option<Vec<String>>,
    /// A script for the shell, at most 1 MiB. Set `exec` or `script`, not both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    /// The shell of the script. Default: `sh`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<ShellName>,
    /// The environment variables: at most 128. A name does not start with `SCYLLA_`.
    #[serde(default, skip_serializing_if = "EnvTable::is_empty")]
    pub env: EnvTable,
    /// The longest run of one attempt, in seconds: 1 to 604800. Default: 3600.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u32>,
    /// The number of new attempts after a failure: 0 to 10. Default: 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retries: Option<u32>,
    /// The work folder, relative to the folder of the job, with no `..` part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// The container of the node. With no container, the node runs as a process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<ContainerFile>,
}

/// The shell of a script.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ShellName {
    /// `sh -e`.
    Sh,
    /// `bash -o pipefail -e`.
    Bash,
}

/// The container of a node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContainerFile {
    /// The image, for example `rust:1.97`.
    pub image: String,
}

/// The environment variables of a node, in the order of the file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvTable(pub Vec<(String, EnvEntry)>);

/// The value of an environment variable: a text, or `{ secret = "NAME" }`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvEntry {
    /// A text in clear.
    Text(String),
    /// The name of a secret of the project.
    Secret(String),
}

impl EnvTable {
    /// Returns `true` if the table has no variable.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Serialize for EnvTable {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (name, entry) in &self.0 {
            map.serialize_entry(name, entry)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for EnvTable {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(EnvTableVisitor)
    }
}

/// Reads a table of variables, keeps its order, and refuses a name two times.
struct EnvTableVisitor;

impl<'de> Visitor<'de> for EnvTableVisitor {
    type Value = EnvTable;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a table of environment variables")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<EnvTable, A::Error> {
        let mut names = HashSet::new();
        let mut vars = Vec::new();
        while let Some(name) = map.next_key::<String>()? {
            if !names.insert(name.clone()) {
                return Err(de::Error::custom(format_args!("duplicate key `{name}`")));
            }
            vars.push((name, map.next_value()?));
        }
        Ok(EnvTable(vars))
    }
}

impl JsonSchema for EnvTable {
    fn schema_name() -> Cow<'static, str> {
        "EnvTable".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "The environment variables, by name.",
            "type": "object",
            "maxProperties": 128,
            "additionalProperties": generator.subschema_for::<EnvEntry>(),
        })
    }
}

impl Serialize for EnvEntry {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Text(text) => serializer.serialize_str(text),
            Self::Secret(name) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("secret", name)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for EnvEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(EnvEntryVisitor)
    }
}

/// Reads a text, or a table with the one key `secret`.
struct EnvEntryVisitor;

impl<'de> Visitor<'de> for EnvEntryVisitor {
    type Value = EnvEntry;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a text, or a table with the key `secret`")
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<EnvEntry, E> {
        Ok(EnvEntry::Text(text.to_owned()))
    }

    fn visit_string<E: de::Error>(self, text: String) -> Result<EnvEntry, E> {
        Ok(EnvEntry::Text(text))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<EnvEntry, A::Error> {
        let mut secret = None;
        while let Some(key) = map.next_key::<String>()? {
            if key != "secret" {
                return Err(de::Error::unknown_field(&key, &["secret"]));
            }
            if secret.is_some() {
                return Err(de::Error::duplicate_field("secret"));
            }
            secret = Some(map.next_value::<String>()?);
        }
        secret
            .map(EnvEntry::Secret)
            .ok_or_else(|| de::Error::missing_field("secret"))
    }
}

impl JsonSchema for EnvEntry {
    fn schema_name() -> Cow<'static, str> {
        "EnvEntry".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A text, or the name of a secret of the project.",
            "anyOf": [
                { "type": "string" },
                {
                    "type": "object",
                    "properties": { "secret": { "type": "string" } },
                    "required": ["secret"],
                    "additionalProperties": false,
                },
            ],
        })
    }
}

/// Returns the JSON Schema of a pipeline file, for the editors of the users.
pub fn json_schema() -> serde_json::Value {
    schemars::schema_for!(PipelineFile).to_value()
}
