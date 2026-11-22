//! The run of one job: the agent fetches its code, waits until each need of a node ended, then
//! runs the node if its needs are open, else skips it. A failure that no `error` need takes stops
//! the job.

mod needs;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;

use scylla_proto::agents::v1::{dispatch_node, Dispatch, DispatchEnv, DispatchNode, EventKind};
use tokio::process::Command;
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::checkout::{self, Failure, Git};
use super::events::{self, Event, Outbox};
use super::step::{self, NodeContext, Outcome};
use super::switch;
use crate::agent::Settings;
use needs::{ended, errors, routed, runs};

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

/// Runs a job until its end, then deletes its work folder. A failed checkout fails the job before
/// the first node. A stop of the server ends it with no more event.
pub(super) async fn run(
    dispatch: Dispatch,
    settings: &Settings,
    git: Option<Git>,
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

    // Fetch the code, then run the nodes in the order of their needs.
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
    let fetched = match &dispatch.checkout {
        Some(spec) => checkout::run(spec, git, &dir, &job, outbox, stopped.clone()).await,
        None => Ok(()),
    };
    let (failure, cancelled) = match fetched {
        Ok(()) => drive(dispatch.nodes, &context, &mut stopped).await,
        Err(Failure::Failed(error)) => {
            for node in &dispatch.nodes {
                skip(&context, &node.id).await;
            }
            (Some(format!("the checkout failed: {error}")), false)
        }
        Err(Failure::Halted) => (None, true),
    };

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
