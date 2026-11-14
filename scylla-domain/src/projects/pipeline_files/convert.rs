//! The conversions between a pipeline file and the nodes of the editor. A file goes through the
//! checks of the nodes (decision 62), and each refusal keeps the path of its value.

use super::document::{ContainerFile, EnvEntry, EnvTable, NodeFile, PipelineFile, ShellName};
use super::{CheckoutSettings, FileKey, FilePipeline, VERSION};
use crate::projects::git_rules::GitRules;
use crate::projects::pipelines::{
    EnvValue, EnvVar, Node, NodeId, Nodes, PipelineName, Runtime, Shell, Step,
};
use crate::projects::secrets::SecretName;
use crate::DomainError;

/// The refusal of a value: its path in the file and what is wrong.
pub(super) type Refusal = (String, String);

impl PipelineFile {
    /// Makes the document of a pipeline, for an export. A default value is not written.
    pub fn of(name: &str, nodes: &[Node], rules: &GitRules, checkout: &CheckoutSettings) -> Self {
        Self {
            version: VERSION,
            name: Some(name.to_owned()),
            on: rules.clone(),
            checkout: checkout.clone(),
            nodes: nodes.iter().map(node_file).collect(),
        }
    }
}

/// Checks a document and makes its pipeline. The name defaults to the key. Returns each refusal
/// that the checks find.
pub(super) fn pipeline(file: PipelineFile, key: &FileKey) -> Result<FilePipeline, Vec<Refusal>> {
    let mut refusals = Vec::new();

    // Check the name and the rules.
    let name = file.name.unwrap_or_else(|| key.as_str().to_owned());
    let name = PipelineName::parse(name)
        .map_err(|error| refusals.push(refusal("name", &error)))
        .ok();
    refusals.extend(
        file.on
            .check("on")
            .into_iter()
            .map(|(path, error)| (path, error.to_string())),
    );

    // Check each node, then the graph when each node passes.
    let mut nodes = Vec::new();
    for (index, node) in file.nodes.into_iter().enumerate() {
        let at = format!("nodes[{index}]");
        let checked = node_of(node, &at)
            .and_then(|node| node.check(&at).map_err(|e| refusal(&e.path, &e.error)));
        match checked {
            Ok(node) => nodes.push(node),
            Err(error) => refusals.push(error),
        }
    }
    let nodes = if refusals.is_empty() {
        Nodes::parse(nodes)
            .map_err(|error| refusals.push(refusal(&error.path, &error.error)))
            .ok()
    } else {
        None
    };

    // Make the pipeline when no check refused a value.
    match (name, nodes) {
        (Some(name), Some(nodes)) if refusals.is_empty() => Ok(FilePipeline {
            name,
            rules: file.on,
            checkout: file.checkout,
            nodes,
        }),
        _ => Err(refusals),
    }
}

/// Makes the refusal of a value from its path and its rule.
fn refusal(path: &str, error: &impl ToString) -> Refusal {
    (path.to_owned(), error.to_string())
}

/// Makes the node of a node of a file, at the path `at`. The checks of the node come after.
fn node_of(node: NodeFile, at: &str) -> Result<Node, Refusal> {
    // Read the id and the needs.
    let id = NodeId::parse(node.id).map_err(|error| refusal(&format!("{at}.id"), &error))?;
    let needs = node
        .needs
        .into_iter()
        .enumerate()
        .map(|(index, need)| {
            NodeId::parse(need).map_err(|error| refusal(&format!("{at}.needs[{index}]"), &error))
        })
        .collect::<Result<_, _>>()?;

    // Read the step: exactly one of exec and script, and a shell only for a script.
    let step = match (node.exec, node.script, node.shell) {
        (Some(argv), None, None) => Step::Exec(argv),
        (Some(_), None, Some(_)) => {
            return Err(refusal(&format!("{at}.shell"), &"is only for a script"));
        }
        (None, Some(source), shell) => Step::Script {
            shell: match shell {
                Some(ShellName::Bash) => Shell::Bash,
                Some(ShellName::Sh) | None => Shell::Sh,
            },
            source,
        },
        _ => return Err(refusal(at, &"must have exactly one of exec and script")),
    };

    // Read the environment, the limits and the runtime.
    let env = env_of(node.env, at)?;
    if node.timeout_seconds == Some(0) {
        let error = DomainError::OutOfRange {
            min: 1,
            max: u64::from(Node::MAX_TIMEOUT),
        };
        return Err(refusal(&format!("{at}.timeout_seconds"), &error));
    }
    Ok(Node {
        id,
        needs,
        step,
        env,
        timeout_seconds: node.timeout_seconds.unwrap_or(Node::DEFAULT_TIMEOUT),
        retries: node.retries.unwrap_or(0),
        working_dir: node.working_dir,
        runtime: match node.container {
            Some(container) => Runtime::Container {
                image: container.image,
            },
            None => Runtime::Process,
        },
    })
}

/// Reads the variables of a node at the path `at`: a secret needs the name of a secret.
fn env_of(env: EnvTable, at: &str) -> Result<Vec<EnvVar>, Refusal> {
    env.0
        .into_iter()
        .enumerate()
        .map(|(index, (name, entry))| {
            let value = match entry {
                EnvEntry::Text(text) => EnvValue::Literal(text),
                EnvEntry::Secret(secret) => EnvValue::Secret(
                    SecretName::parse(secret)
                        .map_err(|error| refusal(&format!("{at}.env[{index}].value"), &error))?,
                ),
            };
            Ok(EnvVar { name, value })
        })
        .collect()
}

/// Makes the node of a file of a node. A default value is not written.
fn node_file(node: &Node) -> NodeFile {
    // Write the step and the environment.
    let (exec, script, shell) = match &node.step {
        Step::Exec(argv) => (Some(argv.clone()), None, None),
        Step::Script { shell, source } => {
            let shell = match shell {
                Shell::Sh => None,
                Shell::Bash => Some(ShellName::Bash),
            };
            (None, Some(source.clone()), shell)
        }
    };
    let env = node
        .env
        .iter()
        .map(|var| {
            let entry = match &var.value {
                EnvValue::Literal(text) => EnvEntry::Text(text.clone()),
                EnvValue::Secret(name) => EnvEntry::Secret(name.as_str().to_owned()),
            };
            (var.name.clone(), entry)
        })
        .collect();

    // Write the other values, with no default.
    NodeFile {
        id: node.id.as_str().to_owned(),
        needs: node
            .needs
            .iter()
            .map(|need| need.as_str().to_owned())
            .collect(),
        exec,
        script,
        shell,
        env: EnvTable(env),
        timeout_seconds: Some(node.timeout_seconds)
            .filter(|seconds| *seconds != Node::DEFAULT_TIMEOUT),
        retries: Some(node.retries).filter(|retries| *retries != 0),
        working_dir: node.working_dir.clone(),
        container: match &node.runtime {
            Runtime::Container { image } => Some(ContainerFile {
                image: image.clone(),
            }),
            Runtime::Process => None,
        },
    }
}
