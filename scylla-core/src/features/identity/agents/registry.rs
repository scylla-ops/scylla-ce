//! The open streams of the agents on this server: at most one stream for each agent.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use scylla_domain::identity::apps::AppId;
use tokio::sync::oneshot;

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
    /// Stops the stream with a reason.
    stop: oneshot::Sender<&'static str>,
}

/// The ticket of an open stream: its number and the signal that stops it.
pub(crate) struct Ticket {
    /// The number of the stream.
    pub(crate) number: u64,
    /// Receives the reason when the server stops the stream.
    pub(crate) stopped: oneshot::Receiver<&'static str>,
}

impl Registry {
    /// Records the stream of an agent, and stops its previous stream.
    pub(crate) fn open(&self, app: AppId) -> Ticket {
        let number = self.next.fetch_add(1, Ordering::Relaxed);
        let (stop, stopped) = oneshot::channel();
        let previous = self.streams().insert(app, Entry { number, stop });
        if let Some(previous) = previous {
            let _ = previous.stop.send("another stream of the agent opened");
        }
        Ticket { number, stopped }
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

    /// Stops the stream of an agent, if it has one.
    pub(crate) fn stop(&self, app: AppId, reason: &'static str) {
        if let Some(entry) = self.streams().remove(&app) {
            let _ = entry.stop.send(reason);
        }
    }

    /// Returns `true` if the agent has an open stream.
    pub(crate) fn is_connected(&self, app: AppId) -> bool {
        self.streams().contains_key(&app)
    }

    /// Locks the map. A panic in another holder leaves the map usable.
    fn streams(&self) -> std::sync::MutexGuard<'_, HashMap<AppId, Entry>> {
        self.streams
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
