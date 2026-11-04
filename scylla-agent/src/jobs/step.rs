//! The attempts of one node: a child process or a container, with a clean environment, a
//! timeout, and a stop of its whole process group.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use scylla_proto::agents::v1::{dispatch_node, DispatchNode, EventKind};
use scylla_proto::pipelines::v1::{LogStream, Shell};
use tokio::process::Command;
use tokio::sync::watch;

use super::events::{self, Event, Outbox};
use super::output;
use super::process::Process;

/// The variables of the agent that a Windows program needs to start.
#[cfg(windows)]
const SYSTEM: [&str; 6] = ["SystemRoot", "windir", "ComSpec", "PATHEXT", "TEMP", "TMP"];

/// What each node of a job shares.
#[derive(Clone)]
pub(super) struct NodeContext {
    /// The job.
    pub(super) job: String,
    /// The pipeline of the job.
    pub(super) pipeline: String,
    /// The project of the pipeline.
    pub(super) project: String,
    /// The work folder of the job.
    pub(super) dir: PathBuf,
    /// The container runtime of the agent.
    pub(super) runtime: Option<String>,
    /// The messages to the server.
    pub(super) outbox: Outbox,
}

/// The end of a node.
pub(super) enum Outcome {
    /// The last attempt completed.
    Completed,
    /// The last attempt failed, with the reason.
    Failed(String),
    /// A failure of another node or a stop of the server stopped it.
    Halted,
}

/// The end of one attempt.
enum Attempt {
    /// The process ended with this code.
    Exited(Option<i32>),
    /// The process did not start, or ran too long.
    Broke(String),
    /// The node must stop.
    Halted,
}

/// Runs the attempts of a node, and sends its events.
pub(super) async fn node(
    context: &NodeContext,
    node: &DispatchNode,
    halted: watch::Receiver<bool>,
) -> Outcome {
    let (job, outbox) = (&context.job, &context.outbox);
    for attempt in 1..=node.retries + 1 {
        // Tell the start of the attempt.
        let kind = if attempt == 1 {
            EventKind::NodeStarted
        } else {
            EventKind::NodeRetrying
        };
        let event = Event {
            node: &node.id,
            attempt,
            ..Event::default()
        };
        events::send(outbox, job, kind, event).await;

        // Run it, and stop at a success, a halt, or the last failure.
        let (exit_code, error) = match once(context, node, attempt, halted.clone()).await {
            Attempt::Halted => return Outcome::Halted,
            Attempt::Exited(Some(0)) => {
                let event = Event {
                    node: &node.id,
                    exit_code: Some(0),
                    ..Event::default()
                };
                events::send(outbox, job, EventKind::NodeCompleted, event).await;
                return Outcome::Completed;
            }
            Attempt::Exited(code) => (
                code,
                format!(
                    "exit code {}",
                    code.map_or("none".to_owned(), |code| code.to_string())
                ),
            ),
            Attempt::Broke(error) => (None, error),
        };
        tracing::warn!(%job, node = %node.id, attempt, %error, "a node attempt failed");
        let line = format!("attempt {attempt} failed: {error}");
        output::report(outbox, job, &node.id, &line).await;
        if attempt == node.retries + 1 {
            let event = Event {
                node: &node.id,
                exit_code,
                error: error.clone(),
                ..Event::default()
            };
            events::send(outbox, job, EventKind::NodeFailed, event).await;
            return Outcome::Failed(error);
        }
    }
    Outcome::Failed("no attempt".to_owned())
}

/// Runs one attempt until its end, its timeout, or a halt.
async fn once(
    context: &NodeContext,
    node: &DispatchNode,
    attempt: u32,
    mut halted: watch::Receiver<bool>,
) -> Attempt {
    // Start the process with its output pipes.
    let name = format!("scylla-{}-{}-{attempt}", context.job, node.id);
    let mut command = match command(context, node, &name).await {
        Ok(command) => command,
        Err(error) => return Attempt::Broke(error),
    };
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    let mut child = match Process::spawn(command) {
        Ok(child) => child,
        Err(error) => return Attempt::Broke(format!("the process did not start: {error}")),
    };
    let masks: Vec<String> = node
        .env
        .iter()
        .filter(|var| var.masked)
        .map(|var| var.value.clone())
        .collect();
    let sink = output::Sink::start(context.outbox.clone(), context.job.clone(), node.id.clone());
    let readers = [
        child
            .stdout()
            .map(|out| output::read(out, LogStream::Stdout, masks.clone(), sink.lines())),
        child
            .stderr()
            .map(|err| output::read(err, LogStream::Stderr, masks, sink.lines())),
    ];

    // Wait for the end, the timeout, or a halt.
    let limit = Duration::from_secs(u64::from(node.timeout_seconds.max(1)));
    let end = tokio::select! {
        status = child.wait() => Attempt::Exited(status.ok().and_then(|status| status.code())),
        () = tokio::time::sleep(limit) => {
            stop(&mut child, context, node, &name).await;
            Attempt::Broke(format!("the attempt ran more than {} seconds", limit.as_secs()))
        }
        () = async { let _ = halted.wait_for(|halted| *halted).await; } => {
            stop(&mut child, context, node, &name).await;
            Attempt::Halted
        }
    };

    // Send the last lines.
    for reader in readers.into_iter().flatten() {
        let _ = reader.await;
    }
    sink.finish().await;
    end
}

/// Stops the processes of an attempt, and its container.
async fn stop(child: &mut Process, context: &NodeContext, node: &DispatchNode, name: &str) {
    if let (Some(runtime), Some(_)) = (&context.runtime, &node.container) {
        let _ = Command::new(runtime).args(["kill", name]).output().await;
    }
    child.stop().await;
}

/// Makes the command of an attempt: a process in the work folder, or a container with the work
/// folder in `/work`. The environment has only the base variables and the variables of the node.
async fn command(
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
/// user profile in the work folder too.
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
    for var in &node.env {
        command.env(&var.name, &var.value);
    }
}
