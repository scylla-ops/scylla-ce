//! The open streams of the agents on this server: at most one stream for each agent, its
//! workspace, its name, its version and its features, and the commands that the server sends
//! to a stream.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use scylla_domain::identity::agents::{fits, AgentLabels, Unfit};
use scylla_domain::identity::apps::AppId;
use scylla_domain::projects::jobs::JobId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use tokio::sync::mpsc;

/// The number of commands that wait for a busy stream.
const COMMANDS: usize = 64;

/// A command of the server to the stream of an agent.
#[derive(Debug)]
pub(crate) enum Command {
    /// End the stream with `ABORTED` and a reason.
    Stop(&'static str),
    /// Tell the agent to stop a job.
    Cancel(JobId),
    /// Look for pending jobs.
    Wake,
}

/// The open streams, in memory: the server is one instance.
#[derive(Default)]
pub(crate) struct Registry {
    /// The open stream of each agent.
    streams: Mutex<HashMap<AppId, Entry>>,
    /// The number of the next stream.
    next: AtomicU64,
}

/// An open stream.
struct Entry {
    /// The number of the stream, so a closed stream never removes a newer one.
    number: u64,
    /// The commands to the stream.
    commands: mpsc::Sender<Command>,
    /// The workspace of the agent.
    workspace: WorkspaceId,
    /// The agent, from its hello.
    agent: Online,
}

/// An agent with an open stream, as its hello tells it.
#[derive(Clone, Debug)]
pub(crate) struct Online {
    /// The display name of the App.
    pub(crate) name: String,
    /// The version of the agent.
    pub(crate) version: String,
    /// Why the agent cannot get the code of a job, or `None` if it can.
    pub(crate) checkout_gap: Option<Unfit>,
    /// `true` if the agent has a container runtime.
    pub(crate) containers: bool,
    /// `true` if the agent runs the conditional nodes.
    pub(crate) conditions: bool,
    /// The labels of the host of the agent.
    pub(crate) automatic_labels: AgentLabels,
    /// The labels that the admins gave to the App.
    pub(crate) labels: AgentLabels,
}

/// The features that a job needs from an agent, or that an agent has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Features {
    /// `true` if the job needs a container runtime.
    pub(crate) containers: bool,
    /// `true` if the job needs the code before its first node.
    pub(crate) checkout: bool,
    /// `true` if the job has a switch, a merge or an `error` need.
    pub(crate) conditions: bool,
}

/// The ticket of an open stream: its number and its commands.
pub(crate) struct Ticket {
    /// The number of the stream.
    pub(crate) number: u64,
    /// The commands. The end of the channel stops the stream.
    pub(crate) commands: mpsc::Receiver<Command>,
}

impl Registry {
    /// Records the stream of an agent of a workspace, and stops its previous stream.
    pub(crate) fn open(&self, app: AppId, workspace: WorkspaceId, agent: Online) -> Ticket {
        let number = self.next.fetch_add(1, Ordering::Relaxed);
        let (sender, commands) = mpsc::channel(COMMANDS);
        let entry = Entry {
            number,
            commands: sender,
            workspace,
            agent,
        };
        if let Some(previous) = self.streams().insert(app, entry) {
            let _ = previous
                .commands
                .try_send(Command::Stop("another stream of the agent opened"));
        }
        Ticket { number, commands }
    }

    /// Forgets a stream that ended, unless a newer stream of the agent replaced it.
    pub(crate) fn close(&self, app: AppId, number: u64) {
        let mut streams = self.streams();
        if streams
            .get(&app)
            .is_some_and(|entry| entry.number == number)
        {
            streams.remove(&app);
        }
    }

    /// Stops the stream of an agent, if it has one. The stream stops even if its channel is
    /// full: the end of the channel stops it too.
    pub(crate) fn stop(&self, app: AppId, reason: &'static str) {
        if let Some(entry) = self.streams().remove(&app) {
            let _ = entry.commands.try_send(Command::Stop(reason));
        }
    }

    /// Tells the stream of an agent to stop a job. A lost command is safe: each later message
    /// of the agent about the job gets a cancel.
    pub(crate) fn cancel(&self, app: AppId, job: JobId) {
        if let Some(entry) = self.streams().get(&app) {
            let _ = entry.commands.try_send(Command::Cancel(job));
        }
    }

    /// Tells each stream to look for pending jobs.
    pub(crate) fn wake_all(&self) {
        for entry in self.streams().values() {
            let _ = entry.commands.try_send(Command::Wake);
        }
    }

    /// Keeps the new labels that the admins gave to an agent with an open stream, and tells each
    /// stream to look for pending jobs: a job can fit the agent now.
    pub(crate) fn relabel(&self, app: AppId, labels: AgentLabels) {
        if let Some(entry) = self.streams().get_mut(&app) {
            entry.agent.labels = labels;
        }
        self.wake_all();
    }

    /// Returns `true` if the agent has an open stream.
    pub(crate) fn is_connected(&self, app: AppId) -> bool {
        self.streams().contains_key(&app)
    }

    /// Returns the agents of a workspace with an open stream.
    pub(crate) fn connected_in(&self, workspace: WorkspaceId) -> Vec<AppId> {
        self.streams()
            .iter()
            .filter(|(_, entry)| entry.workspace == workspace)
            .map(|(app, _)| *app)
            .collect()
    }

    /// Returns `true` if an agent of the workspace with each feature and each label that a job
    /// needs has an open stream.
    pub(crate) fn can_serve(
        &self,
        workspace: WorkspaceId,
        needs: Features,
        labels: &AgentLabels,
    ) -> bool {
        self.streams().values().any(|entry| {
            entry.workspace == workspace
                && (entry.agent.containers || !needs.containers)
                && (entry.agent.checkout_gap.is_none() || !needs.checkout)
                && (entry.agent.conditions || !needs.conditions)
                && fits(labels, &entry.agent.automatic_labels, &entry.agent.labels)
        })
    }

    /// Returns the agents of the workspace with an open stream that cannot get the code of a
    /// job, in the order of their ids.
    pub(crate) fn without_checkout(&self, workspace: WorkspaceId) -> Vec<(AppId, Online)> {
        let mut agents: Vec<(AppId, Online)> = self
            .streams()
            .iter()
            .filter(|(_, entry)| entry.workspace == workspace)
            .filter(|(_, entry)| entry.agent.checkout_gap.is_some())
            .map(|(app, entry)| (*app, entry.agent.clone()))
            .collect();
        agents.sort_by_key(|(app, _)| app.to_string());
        agents
    }

    /// Locks the map. A panic in another holder leaves the map usable.
    fn streams(&self) -> MutexGuard<'_, HashMap<AppId, Entry>> {
        self.streams.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
