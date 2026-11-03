//! The conversions between the proto nodes and the domain nodes. A bad value gives
//! `INVALID_ARGUMENT` with its path, for example `nodes[2].env[0].name`.

use scylla_domain::projects::pipelines::{
    EnvValue, EnvVar, Node, NodeId, Nodes, Pipeline, Runtime, Shell, Step,
};
use scylla_domain::projects::secrets::SecretName;
use scylla_proto::pipelines::v1 as pb;

use crate::api::parse::{self, ToProto};
use crate::Error;

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
    let needs = node
        .needs
        .into_iter()
        .enumerate()
        .map(|(index, need)| {
            NodeId::parse(need).map_err(|e| invalid(format!("{at}.needs[{index}]"), e.to_string()))
        })
        .collect::<Result<_, _>>()?;
    let step = match node.step {
        Some(pb::node::Step::Exec(exec)) => Step::Exec(exec.argv),
        Some(pb::node::Step::Script(script)) => Step::Script {
            shell: shell_of(script.shell()),
            source: script.source,
        },
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
            needs: self
                .needs
                .iter()
                .map(|need| need.as_str().to_owned())
                .collect(),
            step: Some(step(self.step)),
            env,
            timeout_seconds: self.timeout_seconds,
            retries: self.retries,
            working_dir: self.working_dir.unwrap_or_default(),
            container: container(self.runtime),
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
