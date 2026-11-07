//! The run of one job: the agent waits until each need of a node ended, then runs the node if its
//! needs are open, else skips it. A failure that no `error` need takes stops the job.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;

use scylla_proto::agents::v1::{dispatch_node, Dispatch, DispatchEnv, DispatchNode, EventKind};
use scylla_proto::pipelines::v1::Need;
use tokio::process::Command;
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::events::{self, Event, Outbox};
use super::step::{self, NodeContext, Outcome};
use super::switch;
use crate::agent::Settings;

/// The port of a run node that opens when the node failed after its last attempt.
const ERROR: &str = "error";

/// The state of a node in the run.
#[derive(Clone, PartialEq, Eq)]
enum State {
    /// The node waits for its needs.
    Pending,
    /// The node runs.
    Running,
    /// The node completed, with the ports that a switch opened.
    Completed(Vec<String>),
    /// The node failed after its last attempt, with its error.
    Failed(String),
    /// The node did not run, or stopped.
    Skipped,
}

/// Runs a job until its end, then deletes its work folder. A stop of the server ends it with no
/// more event.
pub(super) async fn run(
    dispatch: Dispatch,
    settings: &Settings,
    outbox: &Outbox,
    mut stopped: watch::Receiver<bool>,
) {
    // Make the work folder, then start the job.
    let job = dispatch.job_id.map(|id| id.value).unwrap_or_default();
    let dir = settings.workspace_root.join(&job);
    if let Err(error) = tokio::fs::create_dir_all(dir.join(".scylla")).await {
        let event = Event {
            error: format!("the work folder: {error}"),
            ..Event::default()
        };
        events::send(outbox, &job, EventKind::JobFailed, event).await;
        return;
    }
    events::send(outbox, &job, EventKind::JobStarted, Event::default()).await;

    // Run the nodes in the order of their needs.
    let image = dispatch.nodes.iter().find_map(|node| {
        node.container
            .as_ref()
            .map(|container| container.image.clone())
    });
    let context = NodeContext {
        job: job.clone(),
        pipeline: dispatch.pipeline_id.map(|id| id.value).unwrap_or_default(),
        project: dispatch.project_id.map(|id| id.value).unwrap_or_default(),
        dir: dir.clone(),
        runtime: settings.container_runtime.clone(),
        outbox: outbox.clone(),
    };
    let (failure, cancelled) = drive(dispatch.nodes, &context, &mut stopped).await;

    // Report the end, unless the server stopped the job.
    if !cancelled {
        match failure {
            Some(error) => {
                let event = Event {
                    error,
                    ..Event::default()
                };
                events::send(outbox, &job, EventKind::JobFailed, event).await;
            }
            None => events::send(outbox, &job, EventKind::JobCompleted, Event::default()).await,
        }
    }
    if !settings.keep_workspace {
        let runtime = settings.container_runtime.as_deref();
        remove(dir, runtime.zip(image.as_deref())).await;
    }
}

/// Starts the ready nodes until each node ended. Returns the first failure, and `true` if the
/// server stopped the job.
async fn drive(
    nodes: Vec<DispatchNode>,
    context: &NodeContext,
    stopped: &mut watch::Receiver<bool>,
) -> (Option<String>, bool) {
    let (halt, halted) = watch::channel(false);
    let mut states: HashMap<String, State> = nodes
        .iter()
        .map(|node| (node.id.clone(), State::Pending))
        .collect();
    let mut running = JoinSet::new();
    let (mut failure, mut halting, mut cancelled) = (None, false, false);
    loop {
        // Start or skip each node whose needs ended, until a halt or a stop.
        while !halting && !cancelled {
            let ready = nodes.iter().find(|node| {
                states[&node.id] == State::Pending
                    && node.links.iter().all(|need| ended(&states, need))
            });
            let Some(node) = ready else { break };
            let state = match runs(node, &states) {
                true => begin(context, node, &states, &mut running, &halted).await,
                false => {
                    skip(context, &node.id).await;
                    State::Skipped
                }
            };
            if let State::Failed(error) = &state {
                failure.get_or_insert(format!("the node {} failed: {error}", node.id));
                halting = true;
                let _ = halt.send(true);
            }
            states.insert(node.id.clone(), state);
        }
        if running.is_empty() {
            break;
        }

        // Wait for the end of a node, or for a stop of the server.
        tokio::select! {
            ended = running.join_next() => {
                let Some(Ok((id, outcome))) = ended else { continue };
                let state = match outcome {
                    Outcome::Completed => State::Completed(Vec::new()),
                    Outcome::Failed(error) => {
                        failure.get_or_insert(format!("the node {id} failed: {error}"));
                        if !routed(&nodes, &id) {
                            halting = true;
                            let _ = halt.send(true);
                        }
                        State::Failed(error)
                    }
                    Outcome::Halted => {
                        if !cancelled {
                            skip(context, &id).await;
                        }
                        State::Skipped
                    }
                };
                states.insert(id, state);
            }
            _ = stopped.changed() => {
                cancelled = *stopped.borrow();
                let _ = halt.send(cancelled);
            }
        }
    }

    // Skip the nodes that did not start after a failure that stops the job.
    if halting && !cancelled {
        for node in &nodes {
            if states[&node.id] == State::Pending {
                skip(context, &node.id).await;
            }
        }
    }
    (failure, cancelled)
}

/// Returns `true` if a node whose needs ended runs: a merge with one open need or more, another
/// node with each need open.
fn runs(node: &DispatchNode, states: &HashMap<String, State>) -> bool {
    let open = node.links.iter().filter(|need| open(states, need)).count();
    match node.step {
        Some(dispatch_node::Step::Merge(_)) => open > 0,
        _ => open == node.links.len(),
    }
}

/// Starts a node: a switch and a merge end at once, and a run node starts in a task. A node with
/// an open `error` need gets `SCYLLA_ERROR`. Returns the new state of the node.
async fn begin(
    context: &NodeContext,
    node: &DispatchNode,
    states: &HashMap<String, State>,
    running: &mut JoinSet<(String, Outcome)>,
    halted: &watch::Receiver<bool>,
) -> State {
    let mut node = node.clone();
    if let Some(errors) = errors(&node, states) {
        node.env.push(DispatchEnv {
            name: "SCYLLA_ERROR".to_owned(),
            value: errors,
            masked: false,
        });
    }
    match &node.step {
        Some(dispatch_node::Step::Switch(definition)) => {
            match switch::run(context, &node, definition).await {
                Ok(ports) => State::Completed(ports),
                Err(error) => State::Failed(error),
            }
        }
        Some(dispatch_node::Step::Merge(_)) => {
            switch::merge(context, &node.id).await;
            State::Completed(Vec::new())
        }
        _ => {
            let (context, halted) = (context.clone(), halted.clone());
            running
                .spawn(async move { (node.id.clone(), step::node(&context, &node, halted).await) });
            State::Running
        }
    }
}

/// Returns `true` if the node of a need ended: it completed, failed, or did not run.
fn ended(states: &HashMap<String, State>, need: &Need) -> bool {
    !matches!(
        states.get(&need.node),
        Some(State::Pending | State::Running)
    )
}

/// Returns `true` if a need is open: its node completed with no port asked, a switch opened its
/// port, or a run node failed for the port `error`.
fn open(states: &HashMap<String, State>, need: &Need) -> bool {
    match (states.get(&need.node), need.port.as_str()) {
        (Some(State::Completed(_)), "") => true,
        (Some(State::Completed(ports)), port) => ports.iter().any(|open| open == port),
        (Some(State::Failed(_)), ERROR) => true,
        _ => false,
    }
}

/// Returns one line for each open `error` need of a node, in the order of its needs:
/// `<node>: <error>`. `None` if no `error` need is open.
fn errors(node: &DispatchNode, states: &HashMap<String, State>) -> Option<String> {
    let lines: Vec<String> = node
        .links
        .iter()
        .filter(|need| need.port == ERROR)
        .filter_map(|need| match states.get(&need.node) {
            Some(State::Failed(error)) => Some(format!("{}: {error}", need.node)),
            _ => None,
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Returns `true` if another node takes the failure of a node with an `error` need.
fn routed(nodes: &[DispatchNode], id: &str) -> bool {
    nodes
        .iter()
        .flat_map(|node| &node.links)
        .any(|need| need.node == id && need.port == ERROR)
}

/// Sends the skip of a node.
async fn skip(context: &NodeContext, node: &str) {
    let event = Event {
        node,
        ..Event::default()
    };
    events::send(&context.outbox, &context.job, EventKind::NodeSkipped, event).await;
}

/// Deletes the work folder of a job. A container can write files that this user cannot delete, so
/// after a failure a container of the same image empties the folder. A failure goes to the log.
async fn remove(dir: PathBuf, container: Option<(&str, &str)>) {
    if tokio::fs::remove_dir_all(&dir).await.is_ok() {
        return;
    }
    if let Some((runtime, image)) = container {
        let mount = format!("{}:/work", dir.display());
        let emptied = Command::new(runtime)
            .args(["run", "--rm", "-v", &mount, "--entrypoint", "find", image])
            .args(["/work", "-mindepth", "1", "-delete"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
        if let Err(error) = emptied {
            tracing::warn!(%error, "the container that empties a work folder did not start");
        }
    }
    if let Err(error) = tokio::fs::remove_dir_all(&dir).await {
        tracing::warn!(%error, dir = %dir.display(), "the work folder of a job stays");
    }
}
