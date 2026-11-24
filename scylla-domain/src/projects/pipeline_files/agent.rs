//! The agent settings of a pipeline file: the labels that the agent of a job must have.

use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};

use crate::identity::agents::{AgentLabel, AgentLabels};

/// What the agent of a job of the pipeline must have.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSettings {
    /// The labels that the agent must have, each of them: at most 16. Default: none, so any agent
    /// of the project runs the jobs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(schema_with = "labels_schema")]
    pub labels: Vec<String>,
}

impl AgentSettings {
    /// Makes the settings of the labels of a pipeline.
    pub fn of(labels: &AgentLabels) -> Self {
        Self {
            labels: labels.texts(),
        }
    }

    /// Returns `true` if each setting has its default value.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Returns the JSON Schema of the labels: a list of at most 16 unique labels of their form.
fn labels_schema(_: &mut SchemaGenerator) -> Schema {
    let pattern = format!("^[a-z0-9][a-z0-9._-]{{0,{}}}$", AgentLabel::MAX_LEN - 1);
    json_schema!({
        "description": "The labels that the agent must have, each of them. A label has 1 to 32 \
            lower case letters, digits, dots, dashes or underscores, and starts with a letter or \
            a digit.",
        "type": "array",
        "maxItems": AgentLabels::MAX,
        "uniqueItems": true,
        "items": { "type": "string", "pattern": pattern },
    })
}
