//! The nodes of a pipeline: their needs, a step (a command, a script, a switch or a merge), its
//! environment, its limits and its runtime, and the checks of each value.

use super::needs::{Need, NodeId};
use super::switch::{self, Switch};
use crate::projects::secrets::SecretName;
use crate::DomainError;

/// The error of one value of a node, with the path of the value, for example `nodes[2].env[0]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeError {
    /// The path of the value in the request.
    pub path: String,
    /// The rule that the value breaks.
    pub error: DomainError,
}

impl NodeError {
    /// Makes the error of a value at a path.
    pub fn at(path: impl Into<String>, error: DomainError) -> Self {
        Self {
            path: path.into(),
            error,
        }
    }
}

/// The shell of a script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shell {
    /// `sh -e`.
    Sh,
    /// `bash -o pipefail -e`.
    Bash,
}

/// What a node does. A command and a script make a run node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// A program and its arguments, with no shell.
    Exec(Vec<String>),
    /// A script for a shell.
    Script {
        /// The shell.
        shell: Shell,
        /// The text of the script.
        source: String,
    },
    /// A switch, with no process.
    Switch(Switch),
    /// A merge, with no process.
    Merge,
}

impl Step {
    /// Returns `true` for a step with a process: a command or a script.
    pub fn runs(&self) -> bool {
        matches!(self, Self::Exec(_) | Self::Script { .. })
    }
}

/// The value of an environment variable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvValue {
    /// A text in clear.
    Literal(String),
    /// A secret of the project, by its name. The agent gets its value at the dispatch.
    Secret(SecretName),
}

/// An environment variable of a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvVar {
    /// The name.
    pub name: String,
    /// The value.
    pub value: EnvValue,
}

/// Where a node runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Runtime {
    /// A child process of the agent.
    Process,
    /// A container of an image. Only an agent with a container runtime gets it.
    Container {
        /// The image, for example `rust:1.97`.
        image: String,
    },
}

/// A node of a pipeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// The id, unique in the pipeline.
    pub id: NodeId,
    /// The needs: the node waits until each ended, then runs if they are open.
    pub needs: Vec<Need>,
    /// What the node does.
    pub step: Step,
    /// The environment of the step.
    pub env: Vec<EnvVar>,
    /// The longest run of one attempt, in seconds.
    pub timeout_seconds: u32,
    /// The number of new attempts after a failure.
    pub retries: u32,
    /// The work folder, relative to the folder of the job. `None` is the folder of the job.
    pub working_dir: Option<String>,
    /// Where the node runs.
    pub runtime: Runtime,
}

impl Node {
    /// The default timeout: one hour.
    pub const DEFAULT_TIMEOUT: u32 = 3600;
    /// The longest timeout: one week.
    pub const MAX_TIMEOUT: u32 = 604_800;
    /// The most retries.
    pub const MAX_RETRIES: u32 = 10;
    /// The most environment variables.
    pub const MAX_ENV: usize = 128;
    /// The largest script, in bytes.
    pub const MAX_SCRIPT: usize = 1 << 20;
    /// The largest text of an argument, a variable or an image, in bytes.
    pub const MAX_TEXT: usize = 64 << 10;

    /// Checks each value of a node, at the path `at` of the request. A timeout of 0 gives the
    /// default.
    pub fn check(mut self, at: &str) -> Result<Self, NodeError> {
        // Check the step.
        match &self.step {
            Step::Exec(argv) => {
                if argv.first().is_none_or(|program| program.trim().is_empty()) {
                    return Err(NodeError::at(format!("{at}.exec"), DomainError::Empty));
                }
                for (index, arg) in argv.iter().enumerate() {
                    text(arg, format!("{at}.exec[{index}]"))?;
                }
            }
            Step::Script { source, .. } => {
                if source.trim().is_empty() {
                    return Err(NodeError::at(format!("{at}.script"), DomainError::Empty));
                }
                if source.len() > Self::MAX_SCRIPT || source.contains('\0') {
                    let error = DomainError::TooLarge {
                        max: Self::MAX_SCRIPT,
                    };
                    return Err(NodeError::at(format!("{at}.script"), error));
                }
            }
            Step::Switch(switch) => switch.check(&format!("{at}.switch"))?,
            Step::Merge => {}
        }
        if !self.step.runs() {
            switch::check_flow(&self, at)?;
        }

        // Check the environment: the count, the names and the values.
        if self.env.len() > Self::MAX_ENV {
            let error = DomainError::TooMany { max: Self::MAX_ENV };
            return Err(NodeError::at(format!("{at}.env"), error));
        }
        let mut names = std::collections::HashSet::new();
        for (index, var) in self.env.iter().enumerate() {
            let path = format!("{at}.env[{index}]");
            env_name(&var.name).map_err(|error| NodeError::at(format!("{path}.name"), error))?;
            if !names.insert(var.name.as_str()) {
                return Err(NodeError::at(
                    format!("{path}.name"),
                    DomainError::Duplicate,
                ));
            }
            if let EnvValue::Literal(value) = &var.value {
                text(value, format!("{path}.value"))?;
            }
        }

        // Check the limits, the work folder and the runtime.
        if self.timeout_seconds == 0 {
            self.timeout_seconds = Self::DEFAULT_TIMEOUT;
        }
        if self.timeout_seconds > Self::MAX_TIMEOUT {
            let error = DomainError::OutOfRange {
                min: 1,
                max: Self::MAX_TIMEOUT.into(),
            };
            return Err(NodeError::at(format!("{at}.timeout_seconds"), error));
        }
        if self.retries > Self::MAX_RETRIES {
            let error = DomainError::OutOfRange {
                min: 0,
                max: Self::MAX_RETRIES.into(),
            };
            return Err(NodeError::at(format!("{at}.retries"), error));
        }
        self.working_dir = match self.working_dir.take().filter(|dir| !dir.is_empty()) {
            Some(dir) => {
                Some(relative(dir).map_err(|e| NodeError::at(format!("{at}.working_dir"), e))?)
            }
            None => None,
        };
        if let Runtime::Container { image } = &self.runtime {
            let bad =
                image.is_empty() || image.len() > 255 || image.chars().any(char::is_whitespace);
            if bad {
                let error = DomainError::TooLong { max: 255 };
                return Err(NodeError::at(format!("{at}.container.image"), error));
            }
        }
        Ok(self)
    }
}

/// Checks a text of a node: at most `Node::MAX_TEXT` bytes, and no NUL character.
pub(super) fn text(value: &str, path: String) -> Result<(), NodeError> {
    if value.len() > Node::MAX_TEXT {
        return Err(NodeError::at(
            path,
            DomainError::TooLarge {
                max: Node::MAX_TEXT,
            },
        ));
    }
    if value.contains('\0') {
        return Err(NodeError::at(path, DomainError::ControlCharacter));
    }
    Ok(())
}

/// Checks the name of an environment variable that a node sets: the form, and no `SCYLLA_`.
pub fn env_name(name: &str) -> Result<(), DomainError> {
    var_name(name)?;
    if name.to_ascii_uppercase().starts_with("SCYLLA_") {
        return Err(DomainError::ReservedName);
    }
    Ok(())
}

/// Checks the form of the name of a variable: a letter or an underscore, then letters, digits or
/// underscores, at most 256 bytes.
pub(super) fn var_name(name: &str) -> Result<(), DomainError> {
    let mut chars = name.chars();
    let first = chars.next().ok_or(DomainError::Empty)?;
    let valid = (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name.len() <= 256;
    if !valid {
        return Err(DomainError::NotAnEnvName);
    }
    Ok(())
}

/// Checks that a path is relative and stays inside its root: no root, no `..`, no drive.
fn relative(dir: String) -> Result<String, DomainError> {
    let bad = dir.starts_with('/')
        || dir.starts_with('\\')
        || dir.contains(':')
        || dir.contains('\0')
        || dir.len() > 1024
        || dir.split(['/', '\\']).any(|part| part == "..");
    if bad {
        Err(DomainError::NotRelative)
    } else {
        Ok(dir)
    }
}
