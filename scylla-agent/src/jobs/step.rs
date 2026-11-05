//! The attempts of one node: a child process or a container, with a clean environment, a
//! timeout, and a stop of its whole process group.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use scylla_proto::agents::v1::{dispatch_node, DispatchNode, EventKind};
use scylla_proto::pipelines::v1::LogStream;
use tokio::process::Command;
use tokio::sync::watch;

use super::command;
use super::events::{self, Event, Outbox};
use super::output;
use super::process::Process;

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
    let mut command = match command::build(context, node, &name).await {
        Ok(command) => command,
        Err(error) => return Attempt::Broke(error),
    };
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    let program = command
        .as_std()
        .get_program()
        .to_string_lossy()
        .into_owned();
    let script =
        node.container.is_none() && matches!(node.step, Some(dispatch_node::Step::Script(_)));
    let mut child = match Process::spawn(command) {
        Ok(child) => child,
        Err(error) => return Attempt::Broke(not_started(&program, script, &error)),
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

/// Says which program did not start and why. The error of the system does not name it. On Windows,
/// a script needs a shell that a default install of Git does not put on the path.
fn not_started(program: &str, script: bool, error: &std::io::Error) -> String {
    if error.kind() != std::io::ErrorKind::NotFound {
        return format!("the program {program} did not start: {error}");
    }
    let missing = format!("the agent did not find the program {program} on its PATH");
    if script && cfg!(windows) {
        return format!(
            "{missing}. A script runs with sh or bash. On Windows, install Git for Windows, add \
             its bin folder (for example C:\\Program Files\\Git\\bin) to the PATH, and start the \
             agent again."
        );
    }
    missing
}

/// Stops the processes of an attempt, and its container.
async fn stop(child: &mut Process, context: &NodeContext, node: &DispatchNode, name: &str) {
    if let (Some(runtime), Some(_)) = (&context.runtime, &node.container) {
        let _ = Command::new(runtime).args(["kill", name]).output().await;
    }
    child.stop().await;
}

#[cfg(test)]
mod tests {
    //! Unit tests of the reason of a process that did not start.

    use std::io::{Error, ErrorKind};

    use super::not_started;

    /// The reason names a program that the agent did not find.
    #[test]
    fn a_missing_program_is_named() {
        // given
        let error = Error::from(ErrorKind::NotFound);

        // when
        let reason = not_started("deploy-tool", false, &error);

        // then
        assert_eq!(
            reason,
            "the agent did not find the program deploy-tool on its PATH"
        );
    }

    /// A missing shell says how to fix it on Windows.
    #[cfg(windows)]
    #[test]
    fn a_missing_shell_says_how_to_fix_it() {
        // given
        let error = Error::from(ErrorKind::NotFound);

        // when
        let reason = not_started("sh", true, &error);

        // then
        assert!(reason.starts_with("the agent did not find the program sh on its PATH."));
        assert!(reason.contains("install Git for Windows"), "{reason}");
    }

    /// Another error keeps the words of the system.
    #[test]
    fn another_error_keeps_its_words() {
        // given
        let error = Error::from(ErrorKind::PermissionDenied);

        // when
        let reason = not_started("deploy-tool", true, &error);

        // then
        assert!(reason.starts_with("the program deploy-tool did not start: "));
    }
}
