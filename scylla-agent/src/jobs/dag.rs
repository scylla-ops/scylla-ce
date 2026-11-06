//! The run of one job: the agent starts each node whose needs completed, at once, and stops the
//! job at the first failure.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;

use scylla_proto::agents::v1::{Dispatch, DispatchNode, EventKind};
use tokio::process::Command;
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::events::{self, Event, Outbox};
use super::step::{self, NodeContext, Outcome};
use crate::agent::Settings;

/// The state of a node in the run.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// The node waits for its needs.
    Pending,
    /// The node runs.
    Running,
    /// The node completed.
    Done,
    /// The node failed, or stopped.
    Ended,
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
    let (mut failure, mut cancelled) = (None, false);
    loop {
        // Start each node whose needs completed, until a failure or a stop.
        if failure.is_none() && !cancelled {
            for node in &nodes {
                let ready = states[&node.id] == State::Pending
                    && node
                        .needs
                        .iter()
                        .all(|need| states.get(need) == Some(&State::Done));
                if ready {
                    states.insert(node.id.clone(), State::Running);
                    let (node, context, halted) = (node.clone(), context.clone(), halted.clone());
                    running.spawn(async move {
                        (node.id.clone(), step::node(&context, &node, halted).await)
                    });
                }
            }
        }
        if running.is_empty() {
            break;
        }

        // Wait for the end of a node, or for a stop of the server.
        tokio::select! {
            ended = running.join_next() => {
                let Some(Ok((id, outcome))) = ended else { continue };
                let state = match outcome {
                    Outcome::Completed => State::Done,
                    Outcome::Failed(error) => {
                        failure.get_or_insert(format!("the node {id} failed: {error}"));
                        let _ = halt.send(true);
                        State::Ended
                    }
                    Outcome::Halted => {
                        if !cancelled {
                            skip(context, &id).await;
                        }
                        State::Ended
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

    // Skip the nodes that did not start after a failure.
    if failure.is_some() && !cancelled {
        for node in &nodes {
            if states[&node.id] == State::Pending {
                skip(context, &node.id).await;
            }
        }
    }
    (failure, cancelled)
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
