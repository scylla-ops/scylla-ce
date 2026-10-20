use crate::application::agent::JobDispatch;
use crate::application::agent::dispatch_port::{AgentDispatch, AgentOrder, AgentStream};
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{AppId, JobId, StreamId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::warn;

const ORDER_QUEUE: usize = 64;

struct Conn {
    stream: AgentStream,
    orders: mpsc::Sender<AgentOrder>,
    stop: CancellationToken,
    replaced: Arc<AtomicBool>,
}

/// The half of a connection that the stream keeps: its orders, and `stop`, which fires when
/// the registry drops the connection. `replaced` tells a newer stream of the same app from a
/// disconnect.
pub struct ConnHandle {
    pub stream: AgentStream,
    pub orders: mpsc::Receiver<AgentOrder>,
    pub stop: CancellationToken,
    pub replaced: Arc<AtomicBool>,
}

/// One live connection per app: the newest stream wins. Lock poisoning is recovered: the map
/// is plain data, and a panic here would take down dispatch for the instance.
pub struct InMemoryAgentRegistry {
    agents: Mutex<HashMap<AppId, Conn>>,
    wakes: mpsc::UnboundedSender<Option<AppId>>,
}

impl InMemoryAgentRegistry {
    #[must_use]
    pub fn new(wakes: mpsc::UnboundedSender<Option<AppId>>) -> Self {
        Self {
            agents: Mutex::default(),
            wakes,
        }
    }

    fn agents(&self) -> MutexGuard<'_, HashMap<AppId, Conn>> {
        self.agents
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The new stream replaces the one of its app, and the dispatcher wakes for its agent.
    pub fn register(&self, agent: &AppId) -> ConnHandle {
        let (orders, receiver) = mpsc::channel(ORDER_QUEUE);
        let conn = Conn {
            stream: AgentStream {
                agent: agent.clone(),
                id: StreamId::generate(),
            },
            orders,
            stop: CancellationToken::new(),
            replaced: Arc::new(AtomicBool::new(false)),
        };
        let handle = ConnHandle {
            stream: conn.stream.clone(),
            orders: receiver,
            stop: conn.stop.clone(),
            replaced: conn.replaced.clone(),
        };
        if let Some(older) = self.agents().insert(agent.clone(), conn) {
            warn!(app_id = %agent, "a second stream opened for this app; the older one ends");
            older.replaced.store(true, Ordering::Relaxed);
            older.stop.cancel();
        }
        self.wake(Some(agent));
        handle
    }

    /// Removes the stream if it is still the one of its app.
    pub fn unregister(&self, stream: &AgentStream) {
        let mut agents = self.agents();
        if agents
            .get(&stream.agent)
            .is_some_and(|conn| conn.stream == *stream)
        {
            agents.remove(&stream.agent);
        }
    }

    /// A stream that does not take the order is dropped, so no pass sends to it again.
    fn send(
        &self,
        agent: &AppId,
        stream: Option<&StreamId>,
        order: AgentOrder,
    ) -> DomainResult<()> {
        let mut agents = self.agents();
        let conn = agents
            .get(agent)
            .filter(|conn| stream.is_none_or(|id| conn.stream.id == *id))
            .ok_or_else(|| {
                DomainError::infrastructure(format!("agent {agent} has no such stream"))
            })?;
        if let Err(e) = conn.orders.try_send(order) {
            if let Some(conn) = agents.remove(agent) {
                conn.stop.cancel();
            }
            return Err(DomainError::infrastructure(format!(
                "agent {agent} does not take orders: {e}"
            )));
        }
        Ok(())
    }
}

impl AgentDispatch for InMemoryAgentRegistry {
    fn connected(&self) -> Vec<AgentStream> {
        self.agents()
            .values()
            .map(|conn| conn.stream.clone())
            .collect()
    }

    fn run(&self, stream: &AgentStream, dispatch: JobDispatch) -> DomainResult<()> {
        self.send(&stream.agent, Some(&stream.id), AgentOrder::Run(dispatch))
    }

    fn cancel(&self, agent: &AppId, job_id: &JobId) -> DomainResult<()> {
        self.send(agent, None, AgentOrder::Cancel(job_id.clone()))
    }

    fn disconnect(&self, agent: &AppId) {
        if let Some(conn) = self.agents().remove(agent) {
            conn.stop.cancel();
        }
    }

    fn wake(&self, agent: Option<&AppId>) {
        let _ = self.wakes.send(agent.cloned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> (
        InMemoryAgentRegistry,
        mpsc::UnboundedReceiver<Option<AppId>>,
    ) {
        let (wakes, woken) = mpsc::unbounded_channel();
        (InMemoryAgentRegistry::new(wakes), woken)
    }

    fn app() -> AppId {
        AppId::new("agent-1")
    }

    fn job(id: &str) -> JobId {
        JobId::new(id)
    }

    fn dispatch() -> JobDispatch {
        JobDispatch {
            job_id: "job-1".into(),
            pipeline_id: "pipeline-1".into(),
            nodes: Vec::new(),
        }
    }

    #[test]
    fn a_second_stream_replaces_the_first_and_stops_it() {
        let (registry, _) = registry();
        let first = registry.register(&app());
        let second = registry.register(&app());

        assert!(first.stop.is_cancelled());
        assert!(first.replaced.load(Ordering::Relaxed));
        assert!(!second.stop.is_cancelled());
        assert_ne!(first.stream, second.stream);
        registry.unregister(&first.stream);
        assert_eq!(registry.connected(), std::slice::from_ref(&second.stream));
        registry.unregister(&second.stream);
        assert!(registry.connected().is_empty());
    }

    #[test]
    fn a_disconnect_stops_the_stream_without_marking_it_replaced() {
        let (registry, _) = registry();
        let conn = registry.register(&app());

        registry.disconnect(&app());

        assert!(conn.stop.is_cancelled());
        assert!(!conn.replaced.load(Ordering::Relaxed));
        assert!(registry.connected().is_empty());
    }

    #[tokio::test]
    async fn a_job_goes_to_its_stream_only_and_a_cancel_to_the_open_stream() {
        let (registry, _) = registry();
        let first = registry.register(&app());
        let mut second = registry.register(&app());

        let replaced = registry.run(&first.stream, dispatch());
        registry.run(&second.stream, dispatch()).unwrap();
        registry.cancel(&app(), &job("job-1")).unwrap();
        let missing = registry.cancel(&AppId::new("agent-2"), &job("job-1"));

        assert!(matches!(replaced, Err(DomainError::Infrastructure(_))));
        assert!(matches!(
            second.orders.recv().await,
            Some(AgentOrder::Run(d)) if d.job_id == "job-1"
        ));
        assert!(matches!(
            second.orders.recv().await,
            Some(AgentOrder::Cancel(id)) if id.as_str() == "job-1"
        ));
        assert!(matches!(missing, Err(DomainError::Infrastructure(_))));
    }

    #[test]
    fn a_stream_that_does_not_take_an_order_is_dropped() {
        let (registry, _) = registry();
        let conn = registry.register(&app());
        for i in 0..ORDER_QUEUE {
            registry.cancel(&app(), &job(&format!("job-{i}"))).unwrap();
        }

        let full = registry.run(&conn.stream, dispatch());

        assert!(matches!(full, Err(DomainError::Infrastructure(_))));
        assert!(conn.stop.is_cancelled());
        assert!(registry.connected().is_empty());
    }

    #[test]
    fn a_new_stream_and_a_wake_reach_the_dispatcher() {
        let (registry, mut woken) = registry();

        registry.register(&app());
        registry.wake(None);

        assert_eq!(woken.try_recv().unwrap(), Some(app()));
        assert_eq!(woken.try_recv().unwrap(), None);
    }
}
