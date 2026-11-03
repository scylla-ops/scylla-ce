//! The nodes of a pipeline of the old product, read into the nodes of this base. The old
//! product had no container: each node is a process.

use scylla_domain::projects::pipelines::{
    EnvValue, EnvVar, Node, NodeId, Nodes, Runtime, Shell, Step,
};
use scylla_domain::projects::secrets::SecretName;
use serde::Deserialize;
use serde_json::Value;

/// A node in the JSON of the old product.
#[derive(Deserialize)]
struct OldNode {
    /// The id.
    id: String,
    /// The needs.
    #[serde(default)]
    deps: Vec<String>,
    /// The work folder.
    #[serde(default)]
    working_dir: Option<String>,
    /// The environment.
    #[serde(default)]
    env: Vec<OldEnv>,
    /// The step.
    step: OldStep,
    /// The timeout, in seconds.
    #[serde(default)]
    timeout_seconds: Option<u32>,
    /// The retries.
    #[serde(default)]
    retries: u32,
}

/// A step in the JSON of the old product.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum OldStep {
    /// A program and its arguments.
    Exec {
        /// The program.
        command: String,
        /// The arguments.
        #[serde(default)]
        args: Vec<String>,
    },
    /// A script.
    Script {
        /// The text.
        script: String,
        /// `sh` or `bash`.
        #[serde(default)]
        shell: Option<String>,
    },
}

/// A variable in the JSON of the old product.
#[derive(Deserialize)]
struct OldEnv {
    /// The name.
    key: String,
    /// A literal or the name of a secret.
    source: OldSource,
}

/// The value of a variable in the JSON of the old product.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum OldSource {
    /// A text.
    Literal(String),
    /// The name of a secret.
    Secret(String),
}

/// Reads and checks the nodes of an old pipeline. Returns the reason of a refusal.
pub(super) fn read(value: Value) -> Result<Nodes, String> {
    let old: Vec<OldNode> =
        serde_json::from_value(value).map_err(|error| format!("bad JSON: {error}"))?;
    let nodes = old.into_iter().map(node).collect::<Result<Vec<_>, _>>()?;
    Nodes::parse(nodes).map_err(|error| format!("{}: {}", error.path, error.error))
}

/// Reads one old node.
fn node(old: OldNode) -> Result<Node, String> {
    let id =
        |text: String| NodeId::parse(text.clone()).map_err(|error| format!("node {text}: {error}"));
    let step = match old.step {
        OldStep::Exec { command, args } => {
            Step::Exec(std::iter::once(command).chain(args).collect())
        }
        OldStep::Script { script, shell } => Step::Script {
            shell: if shell.as_deref() == Some("bash") {
                Shell::Bash
            } else {
                Shell::Sh
            },
            source: script,
        },
    };
    let env = old
        .env
        .into_iter()
        .map(|var| {
            let value = match var.source {
                OldSource::Literal(text) => EnvValue::Literal(text),
                OldSource::Secret(name) => EnvValue::Secret(
                    SecretName::parse(name.clone())
                        .map_err(|error| format!("secret {name}: {error}"))?,
                ),
            };
            Ok(EnvVar {
                name: var.key,
                value,
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(Node {
        id: id(old.id)?,
        needs: old.deps.into_iter().map(id).collect::<Result<_, _>>()?,
        step,
        env,
        timeout_seconds: old.timeout_seconds.unwrap_or(0),
        retries: old.retries,
        working_dir: old.working_dir.filter(|dir| !dir.is_empty() && dir != "."),
        runtime: Runtime::Process,
    })
}
