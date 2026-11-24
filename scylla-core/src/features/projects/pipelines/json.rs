//! The JSON form of the nodes of a pipeline, of their places and of the state of the nodes of a
//! job, as the `jsonb` columns keep them.

use std::collections::BTreeMap;

use scylla_domain::projects::jobs::{NodeRun, NodeStatus};
use scylla_domain::projects::pipelines::{
    EnvValue, EnvVar, Node, NodeId, PipelineLayout, PortName, Position, Runtime, Shell, Step,
};
use scylla_domain::projects::secrets::SecretName;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use super::switch::{NeedJson, SwitchJson};
use crate::Error;

/// A node in JSON.
#[derive(Serialize, Deserialize)]
struct NodeJson {
    /// The id.
    id: String,
    /// The needs.
    needs: Vec<NeedJson>,
    /// The step.
    step: StepJson,
    /// The environment.
    env: Vec<EnvJson>,
    /// The timeout in seconds.
    timeout_seconds: u32,
    /// The retries.
    retries: u32,
    /// The work folder.
    working_dir: Option<String>,
    /// The image of the container, or `None` for a process.
    image: Option<String>,
}

/// A step in JSON.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StepJson {
    /// A program and its arguments.
    Exec {
        /// The program and its arguments.
        argv: Vec<String>,
    },
    /// A script.
    Script {
        /// `true` for bash, `false` for sh.
        bash: bool,
        /// The text of the script.
        source: String,
    },
    /// A switch.
    Switch(SwitchJson),
    /// A merge.
    Merge,
}

/// An environment variable in JSON: a literal or the name of a secret.
#[derive(Serialize, Deserialize)]
struct EnvJson {
    /// The name.
    name: String,
    /// The text of a literal.
    literal: Option<String>,
    /// The name of a secret.
    secret: Option<String>,
}

/// The state of a node in JSON. The times are unix microseconds.
#[derive(Serialize, Deserialize)]
struct RunJson {
    /// The node.
    node: String,
    /// The state.
    status: String,
    /// The attempt.
    attempt: u32,
    /// The start.
    started_at: Option<i64>,
    /// The end.
    finished_at: Option<i64>,
    /// The exit code.
    exit_code: Option<i32>,
    /// The error.
    error: Option<String>,
    /// The ports that a switch opened.
    ports: Vec<String>,
}

/// Returns the JSON of nodes.
pub(crate) fn nodes(nodes: &[Node]) -> Value {
    let items: Vec<NodeJson> = nodes.iter().map(node_json).collect();
    serde_json::to_value(items).unwrap_or(Value::Null)
}

/// Reads nodes from their JSON. Bad data in the database gives `INTERNAL`.
pub(crate) fn read_nodes(value: Value) -> Result<Vec<Node>, Error> {
    let items: Vec<NodeJson> = serde_json::from_value(value).map_err(bad)?;
    items.into_iter().map(read_node).collect()
}

/// A place in JSON.
#[derive(Serialize, Deserialize)]
struct PlaceJson {
    /// The horizontal place.
    x: i32,
    /// The vertical place.
    y: i32,
}

/// Returns the JSON of one place.
pub(crate) fn place(place: Position) -> Value {
    let (x, y) = (place.x, place.y);
    serde_json::to_value(PlaceJson { x, y }).unwrap_or(Value::Null)
}

/// Returns the JSON of the places of nodes, by the id of the node.
pub(crate) fn places(places: &BTreeMap<NodeId, Position>) -> Value {
    let items: BTreeMap<&str, Value> = places
        .iter()
        .map(|(id, position)| (id.as_str(), place(*position)))
        .collect();
    serde_json::to_value(items).unwrap_or(Value::Null)
}

/// Reads saved places from their JSON. Bad data in the database gives `INTERNAL`.
pub(crate) fn read_layout(start: Option<Value>, nodes: Value) -> Result<PipelineLayout, Error> {
    let position = |json: PlaceJson| Position {
        x: json.x,
        y: json.y,
    };
    let start: Option<PlaceJson> = start.map(serde_json::from_value).transpose().map_err(bad)?;
    let items: BTreeMap<String, PlaceJson> = serde_json::from_value(nodes).map_err(bad)?;
    let mut nodes = BTreeMap::new();
    for (id, json) in items {
        nodes.insert(NodeId::parse(id).map_err(bad)?, position(json));
    }
    Ok(PipelineLayout {
        start: start.map(position),
        nodes,
    })
}

/// Returns the JSON of the states of nodes.
pub(crate) fn runs(runs: &[NodeRun]) -> Value {
    let items: Vec<RunJson> = runs
        .iter()
        .map(|run| RunJson {
            node: run.node.as_str().to_owned(),
            status: run.status.as_str().to_owned(),
            attempt: run.attempt,
            started_at: run.started_at.map(micros),
            finished_at: run.finished_at.map(micros),
            exit_code: run.exit_code,
            error: run.error.clone(),
            ports: run
                .ports
                .iter()
                .map(|port| port.as_str().to_owned())
                .collect(),
        })
        .collect();
    serde_json::to_value(items).unwrap_or(Value::Null)
}

/// Reads the states of nodes from their JSON.
pub(crate) fn read_runs(value: Value) -> Result<Vec<NodeRun>, Error> {
    let items: Vec<RunJson> = serde_json::from_value(value).map_err(bad)?;
    items
        .into_iter()
        .map(|run| {
            Ok(NodeRun {
                node: NodeId::parse(run.node).map_err(bad)?,
                status: NodeStatus::parse(&run.status).ok_or_else(|| bad("status"))?,
                attempt: run.attempt,
                started_at: run.started_at.map(time_of).transpose()?,
                finished_at: run.finished_at.map(time_of).transpose()?,
                exit_code: run.exit_code,
                error: run.error,
                ports: run
                    .ports
                    .into_iter()
                    .map(|port| PortName::parse(port).map_err(bad))
                    .collect::<Result<_, _>>()?,
            })
        })
        .collect()
}

/// Returns the JSON of one node.
fn node_json(node: &Node) -> NodeJson {
    let step = match &node.step {
        Step::Exec(argv) => StepJson::Exec { argv: argv.clone() },
        Step::Script { shell, source } => StepJson::Script {
            bash: *shell == Shell::Bash,
            source: source.clone(),
        },
        Step::Switch(definition) => StepJson::Switch(SwitchJson::of(definition)),
        Step::Merge => StepJson::Merge,
    };
    let env = node
        .env
        .iter()
        .map(|var| match &var.value {
            EnvValue::Literal(text) => EnvJson {
                name: var.name.clone(),
                literal: Some(text.clone()),
                secret: None,
            },
            EnvValue::Secret(name) => EnvJson {
                name: var.name.clone(),
                literal: None,
                secret: Some(name.as_str().to_owned()),
            },
        })
        .collect();
    NodeJson {
        id: node.id.as_str().to_owned(),
        needs: node.needs.iter().map(NeedJson::of).collect(),
        step,
        env,
        timeout_seconds: node.timeout_seconds,
        retries: node.retries,
        working_dir: node.working_dir.clone(),
        image: match &node.runtime {
            Runtime::Process => None,
            Runtime::Container { image } => Some(image.clone()),
        },
    }
}

/// Reads one node from its JSON.
fn read_node(json: NodeJson) -> Result<Node, Error> {
    let step = match json.step {
        StepJson::Exec { argv } => Step::Exec(argv),
        StepJson::Script { bash, source } => Step::Script {
            shell: if bash { Shell::Bash } else { Shell::Sh },
            source,
        },
        StepJson::Switch(definition) => Step::Switch(definition.read()?),
        StepJson::Merge => Step::Merge,
    };
    let env = json
        .env
        .into_iter()
        .map(|var| {
            let value = match (var.literal, var.secret) {
                (Some(text), None) => EnvValue::Literal(text),
                (None, Some(name)) => EnvValue::Secret(SecretName::parse(name).map_err(bad)?),
                _ => return Err(bad("env")),
            };
            Ok(EnvVar {
                name: var.name,
                value,
            })
        })
        .collect::<Result<_, Error>>()?;
    Ok(Node {
        id: NodeId::parse(json.id).map_err(bad)?,
        needs: json
            .needs
            .into_iter()
            .map(NeedJson::read)
            .collect::<Result<_, _>>()?,
        step,
        env,
        timeout_seconds: json.timeout_seconds,
        retries: json.retries,
        working_dir: json.working_dir,
        runtime: match json.image {
            Some(image) => Runtime::Container { image },
            None => Runtime::Process,
        },
    })
}

/// Returns a time as unix microseconds.
fn micros(at: OffsetDateTime) -> i64 {
    (at.unix_timestamp_nanos() / 1_000) as i64
}

/// Reads a time from unix microseconds.
fn time_of(micros: i64) -> Result<OffsetDateTime, Error> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1_000).map_err(bad)
}

/// The error of bad JSON in the database.
pub(super) fn bad(error: impl std::fmt::Display) -> Error {
    Error::Internal(format!("bad node data in the database: {error}"))
}
