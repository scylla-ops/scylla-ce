//! The command of an attempt: a process or a container, with a clean environment.

use std::path::Path;

use scylla_proto::agents::v1::{dispatch_node, DispatchNode};
use scylla_proto::pipelines::v1::Shell;
use tokio::process::Command;

use super::step::NodeContext;

/// The variables of the agent that a Windows program needs to start.
#[cfg(windows)]
const SYSTEM: [&str; 6] = ["SystemRoot", "windir", "ComSpec", "PATHEXT", "TEMP", "TMP"];

/// The variables that tell the program of a container runtime where its daemon is. The program
/// gets their values, and the container gets only the names of `base_names` and of the node.
const RUNTIME: [&str; 4] = [
    "DOCKER_HOST",
    "DOCKER_CONFIG",
    "CONTAINER_HOST",
    "XDG_RUNTIME_DIR",
];

/// Makes the command of an attempt: a process in the work folder, or a container with the work
/// folder in `/work`. The environment has only the base variables and the variables of the node.
pub(super) async fn build(
    context: &NodeContext,
    node: &DispatchNode,
    name: &str,
) -> Result<Command, String> {
    // Find the work folder, and write a script to a file.
    let work = context.dir.join(&node.working_dir);
    tokio::fs::create_dir_all(&work)
        .await
        .map_err(|error| format!("the work folder: {error}"))?;
    let script = match &node.step {
        Some(dispatch_node::Step::Script(script)) => {
            let file = format!(".scylla/{}.sh", node.id);
            tokio::fs::write(context.dir.join(&file), &script.source)
                .await
                .map_err(|error| format!("the script: {error}"))?;
            let shell = match script.shell() {
                Shell::Bash => vec!["bash", "-o", "pipefail", "-e"],
                Shell::Sh | Shell::Unspecified => vec!["sh", "-e"],
            };
            Some((shell, file))
        }
        _ => None,
    };
    let argv: Vec<String> = match (&node.step, &script) {
        (Some(dispatch_node::Step::Exec(exec)), _) => exec.argv.clone(),
        (_, Some((shell, _))) => shell.iter().map(|part| (*part).to_owned()).collect(),
        _ => return Err("the node has no step".to_owned()),
    };

    // Build a process, or a container of the image.
    let mut command = match (&node.container, &context.runtime) {
        (None, _) => {
            let mut command = Command::new(&argv[0]);
            command.args(&argv[1..]).current_dir(&work);
            if let Some((_, file)) = &script {
                command.arg(context.dir.join(file));
            }
            command
        }
        (Some(container), Some(runtime)) => {
            let mut command = Command::new(runtime);
            let mount = format!("{}:/work", context.dir.display());
            let workdir = format!("/work/{}", node.working_dir);
            command.args(["run", "--rm", "--name", name, "-v", &mount, "-w", &workdir]);
            for var in base_names()
                .into_iter()
                .chain(node.env.iter().map(|var| var.name.as_str()))
            {
                command.args(["-e", var]);
            }
            command.arg(&container.image).args(&argv);
            if let Some((_, file)) = &script {
                command.arg(format!("/work/{file}"));
            }
            command
        }
        (Some(_), None) => return Err("the agent has no container runtime".to_owned()),
    };
    environment(&mut command, context, node);
    Ok(command)
}

/// The names of the base variables that a container gets.
fn base_names() -> Vec<&'static str> {
    vec![
        "CI",
        "SCYLLA_JOB_ID",
        "SCYLLA_NODE_ID",
        "SCYLLA_PIPELINE_ID",
        "SCYLLA_PROJECT_ID",
    ]
}

/// Sets a clean environment: the path, a home in the work folder, the base variables of the
/// job, then the variables of the node. On Windows, it keeps the system variables and puts the
/// user profile in the work folder too. A container step also keeps the settings of its runtime.
fn environment(command: &mut Command, context: &NodeContext, node: &DispatchNode) {
    command.env_clear();
    #[cfg(windows)]
    {
        for name in SYSTEM {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command.env("USERPROFILE", &context.dir);
    }
    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".to_owned());
    command
        .env("PATH", path)
        .env("HOME", &context.dir)
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .env("TERM", "dumb")
        .env("CI", "true")
        .env("SCYLLA_JOB_ID", &context.job)
        .env("SCYLLA_NODE_ID", &node.id)
        .env("SCYLLA_PIPELINE_ID", &context.pipeline)
        .env("SCYLLA_PROJECT_ID", &context.project);
    if node.container.is_some() {
        runtime_settings(command);
    }
    for var in &node.env {
        command.env(&var.name, &var.value);
    }
}

/// Keeps the settings of the container runtime of the agent. The home of the command is the work
/// folder, so `DOCKER_CONFIG` shows the folder of the settings in the home of the agent.
fn runtime_settings(command: &mut Command) {
    for name in RUNTIME {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    if std::env::var_os("DOCKER_CONFIG").is_none() {
        let settings = std::env::var_os("HOME").map(|home| Path::new(&home).join(".docker"));
        if let Some(settings) = settings.filter(|folder| folder.is_dir()) {
            command.env("DOCKER_CONFIG", settings);
        }
    }
}
