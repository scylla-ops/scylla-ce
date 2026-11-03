//! The open streams of the agents on this server: at most one stream for each agent, and the
//! commands that the server sends to a stream.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use scylla_domain::identity::apps::AppId;
use scylla_domain::projects::jobs::JobId;
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
}

/// The ticket of an open stream: its number and its commands.
pub(crate) struct Ticket {
    /// The number of the stream.
    pub(crate) number: u64,
    /// The commands. The end of the channel stops the stream.
    pub(crate) commands: mpsc::Receiver<Command>,
}

impl Registry {
    /// Records the stream of an agent, and stops its previous stream.
    pub(crate) fn open(&self, app: AppId) -> Ticket {
        let number = self.next.fetch_add(1, Ordering::Relaxed);
        let (sender, commands) = mpsc::channel(COMMANDS);
        let entry = Entry {
            number,
            commands: sender,
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

    /// Returns `true` if the agent has an open stream.
    pub(crate) fn is_connected(&self, app: AppId) -> bool {
        self.streams().contains_key(&app)
    }

    /// Locks the map. A panic in another holder leaves the map usable.
    fn streams(&self) -> MutexGuard<'_, HashMap<AppId, Entry>> {
        self.streams.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
