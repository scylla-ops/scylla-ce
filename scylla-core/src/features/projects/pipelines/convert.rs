//! The conversions between the proto nodes and places and the domain nodes and places. A bad
//! value gives `INVALID_ARGUMENT` with its path, for example `nodes[2].env[0].name`.

use std::collections::BTreeMap;

use scylla_domain::projects::pipelines::{
    EnvValue, EnvVar, Node, NodeId, Nodes, Pipeline, PipelineLayout, Position, Runtime, Shell, Step,
};
use scylla_domain::projects::secrets::SecretName;
use scylla_domain::DomainError;
use scylla_proto::pipelines::v1 as pb;

use super::switch;
use crate::api::parse::{self, ToProto};
use crate::Error;

/// Reads and checks the places of a request. A key that cannot be the id of a node is ignored, as
/// each id that is not a node of the pipeline is.
pub(crate) fn layout(layout: pb::PipelineLayout) -> Result<PipelineLayout, Error> {
    if layout.nodes.len() > PipelineLayout::MAX_NODES {
        let max = PipelineLayout::MAX_NODES;
        let error = DomainError::TooMany { max };
        return Err(Error::invalid("layout.nodes", error.to_string()));
    }
    let start = layout
        .start
        .map(|place| position(place, "layout.start"))
        .transpose()?;
    let mut nodes = BTreeMap::new();
    for (id, place) in layout.nodes {
        if let Ok(id) = NodeId::parse(id) {
            let place = position(place, &format!("layout.nodes[{}]", id.as_str()))?;
            nodes.insert(id, place);
        }
    }
    Ok(PipelineLayout { start, nodes })
}

/// Reads one place of a request, at the path `at`.
fn position(place: pb::Position, at: &str) -> Result<Position, Error> {
    Position::parse(place.x, place.y).map_err(|error| Error::invalid(at, error.to_string()))
}

/// Reads and checks the nodes of a request.
pub(crate) fn nodes(nodes: Vec<pb::Node>) -> Result<Nodes, Error> {
    let nodes = nodes
        .into_iter()
        .enumerate()
        .map(|(index, node)| node_of(node, &format!("nodes[{index}]")))
        .collect::<Result<Vec<_>, _>>()?;
    Nodes::parse(nodes).map_err(|error| Error::invalid(error.path, error.error.to_string()))
}

/// Reads one node of a request, at the path `at`.
fn node_of(node: pb::Node, at: &str) -> Result<Node, Error> {
    let invalid = |field: String, reason: String| Error::invalid(field, reason);
    let id = NodeId::parse(node.id).map_err(|e| invalid(format!("{at}.id"), e.to_string()))?;
    let needs = switch::needs_of(node.needs, at)?;
    let step = match node.step {
        Some(pb::node::Step::Exec(exec)) => Step::Exec(exec.argv),
        Some(pb::node::Step::Script(script)) => Step::Script {
            shell: shell_of(script.shell()),
            source: script.source,
        },
        Some(pb::node::Step::Switch(definition)) => {
            Step::Switch(switch::switch_of(definition, &format!("{at}.switch"))?)
        }
        Some(pb::node::Step::Merge(_)) => Step::Merge,
        None => return Err(invalid(format!("{at}.step"), "is required".to_owned())),
    };
    let env = node
        .env
        .into_iter()
        .enumerate()
        .map(|(index, var)| env_of(var, &format!("{at}.env[{index}]")))
        .collect::<Result<_, _>>()?;
    Ok(Node {
        id,
        needs,
        step,
        env,
        timeout_seconds: node.timeout_seconds,
        retries: node.retries,
        working_dir: Some(node.working_dir).filter(|dir| !dir.is_empty()),
        runtime: match node.container {
            Some(container) => Runtime::Container {
                image: container.image,
            },
            None => Runtime::Process,
        },
    })
}

/// Reads one environment variable of a request, at the path `at`.
fn env_of(var: pb::EnvVar, at: &str) -> Result<EnvVar, Error> {
    let value = match var.value {
        Some(pb::env_var::Value::Literal(text)) => EnvValue::Literal(text),
        Some(pb::env_var::Value::Secret(name)) => EnvValue::Secret(
            SecretName::parse(name)
                .map_err(|error| Error::invalid(format!("{at}.secret"), error.to_string()))?,
        ),
        None => return Err(Error::invalid(format!("{at}.value"), "is required")),
    };
    Ok(EnvVar {
        name: var.name,
        value,
    })
}

/// Reads a shell. Not set gives sh.
fn shell_of(shell: pb::Shell) -> Shell {
    match shell {
        pb::Shell::Bash => Shell::Bash,
        pb::Shell::Sh | pb::Shell::Unspecified => Shell::Sh,
    }
}

/// Returns the proto of a step.
pub(crate) fn step(step: Step) -> pb::node::Step {
    match step {
        Step::Exec(argv) => pb::node::Step::Exec(pb::Exec { argv }),
        Step::Script { shell, source } => pb::node::Step::Script(script(shell, source)),
        Step::Switch(definition) => pb::node::Step::Switch(switch::switch(definition)),
        Step::Merge => pb::node::Step::Merge(pb::Merge {}),
    }
}

/// Returns the proto of a script.
pub(crate) fn script(shell: Shell, source: String) -> pb::Script {
    let shell = match shell {
        Shell::Sh => pb::Shell::Sh,
        Shell::Bash => pb::Shell::Bash,
    };
    pb::Script {
        shell: shell.into(),
        source,
    }
}

/// Returns the proto of a runtime.
pub(crate) fn container(runtime: Runtime) -> Option<pb::Container> {
    match runtime {
        Runtime::Process => None,
        Runtime::Container { image } => Some(pb::Container { image }),
    }
}

impl ToProto<pb::Node> for Node {
    fn to_proto(self) -> pb::Node {
        let env = self
            .env
            .into_iter()
            .map(|var| pb::EnvVar {
                name: var.name,
                value: Some(match var.value {
                    EnvValue::Literal(text) => pb::env_var::Value::Literal(text),
                    EnvValue::Secret(name) => pb::env_var::Value::Secret(name.as_str().to_owned()),
                }),
            })
            .collect();
        pb::Node {
            id: self.id.as_str().to_owned(),
            needs: switch::needs(self.needs),
            step: Some(step(self.step)),
            env,
            timeout_seconds: self.timeout_seconds,
            retries: self.retries,
            working_dir: self.working_dir.unwrap_or_default(),
            container: container(self.runtime),
        }
    }
}

impl ToProto<pb::Position> for Position {
    fn to_proto(self) -> pb::Position {
        pb::Position {
            x: self.x,
            y: self.y,
        }
    }
}

impl ToProto<pb::PipelineLayout> for PipelineLayout {
    fn to_proto(self) -> pb::PipelineLayout {
        pb::PipelineLayout {
            start: self.start.map(ToProto::to_proto),
            nodes: self
                .nodes
                .into_iter()
                .map(|(id, place)| (id.as_str().to_owned(), place.to_proto()))
                .collect(),
        }
    }
}

impl ToProto<pb::Pipeline> for Pipeline {
    fn to_proto(self) -> pb::Pipeline {
        pb::Pipeline {
            id: Some(self.id.to_proto()),
            project_id: Some(self.project_id.to_proto()),
            workspace_id: Some(self.workspace_id.to_proto()),
            created_by: Some(self.created_by.to_proto()),
            name: self.name,
            nodes: self.nodes.into_iter().map(ToProto::to_proto).collect(),
            version: self.version,
            created_at: Some(parse::timestamp(self.created_at)),
            updated_at: Some(parse::timestamp(self.updated_at)),
        }
    }
}
