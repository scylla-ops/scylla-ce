//! The running jobs of the agent, each with the signal that stops it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use scylla_proto::agents::v1::Dispatch;
use tokio::sync::watch;

use super::dag;
use super::events::Outbox;
use crate::agent::Settings;

/// The running jobs. A clone shares the same jobs.
#[derive(Clone, Default)]
pub(crate) struct JobSet {
    /// The stop signal of each running job, by its id.
    jobs: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
}

impl JobSet {
    /// Starts a job in a task, unless it runs already. The job leaves the set at its end.
    pub(crate) fn start(&self, dispatch: Dispatch, settings: Settings, outbox: Outbox) {
        let Some(id) = dispatch.job_id.as_ref().map(|id| id.value.clone()) else {
            return;
        };
        let (stop, stopped) = watch::channel(false);
        {
            let mut jobs = self.jobs();
            if jobs.contains_key(&id) {
                return;
            }
            jobs.insert(id.clone(), stop);
        }
        let set = self.clone();
        tokio::spawn(async move {
            dag::run(dispatch, &settings, &outbox, stopped).await;
            set.jobs().remove(&id);
        });
    }

    /// Stops a running job. Its nodes stop, and it sends no more event.
    pub(crate) fn cancel(&self, id: &str) {
        if let Some(stop) = self.jobs().get(id) {
            let _ = stop.send(true);
        }
    }

    /// Stops each running job.
    pub(crate) fn cancel_all(&self) {
        for stop in self.jobs().values() {
            let _ = stop.send(true);
        }
    }

    /// Returns the ids of the running jobs.
    pub(crate) fn running(&self) -> Vec<String> {
        self.jobs().keys().cloned().collect()
    }

    /// Returns `true` while a job runs.
    pub(crate) fn is_empty(&self) -> bool {
        self.jobs().is_empty()
    }

    /// Locks the map. A panic in another holder leaves the map usable.
    fn jobs(&self) -> MutexGuard<'_, HashMap<String, watch::Sender<bool>>> {
        self.jobs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
