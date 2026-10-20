use crate::application::agent::dispatch_use_case::{DispatchPendingJobs, DispatchUseCases};
use crate::domain::caller::{CallerContext, ServiceIdentity};
use crate::domain::ids::AppId;
use scylla_extension::Actions;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, instrument, warn};

/// Sends `DispatchPendingJobs` for the agents that the wakes name.
pub struct PendingJobScheduler {
    actions: Arc<Actions>,
    dispatch_uc: Arc<DispatchUseCases>,
}

impl PendingJobScheduler {
    #[must_use]
    pub fn new(actions: Arc<Actions>, dispatch_uc: Arc<DispatchUseCases>) -> Self {
        Self {
            actions,
            dispatch_uc,
        }
    }

    /// The first wake and every wake already queued, as one pass: `None` (every agent) wins
    /// over a list of agents.
    pub fn targets(
        first: Option<AppId>,
        queued: &mut mpsc::UnboundedReceiver<Option<AppId>>,
    ) -> Option<Vec<AppId>> {
        let wakes = std::iter::once(first).chain(std::iter::from_fn(|| queued.try_recv().ok()));
        let mut targets = Some(Vec::new());
        for wake in wakes {
            match (wake, &mut targets) {
                (None, _) => targets = None,
                (Some(agent), Some(agents)) if !agents.contains(&agent) => agents.push(agent),
                _ => {}
            }
        }
        targets
    }

    #[instrument(skip(self))]
    pub async fn drain(&self, agents: Option<Vec<AppId>>) -> usize {
        let caller = CallerContext::Service(ServiceIdentity::job_dispatcher());
        match self
            .actions
            .run(&*self.dispatch_uc, &caller, DispatchPendingJobs { agents })
            .await
        {
            Ok(placed) if placed.is_empty() => 0,
            Ok(placed) => {
                info!(dispatched = placed.len(), "placed pending jobs on agents");
                placed.len()
            }
            Err(e) => {
                warn!(error = %e, "the dispatch pass failed");
                0
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::ProjectId;
    use crate::test_support::authz::{RecordingPermissionService, actions};
    use crate::test_support::jobs::job;
    use crate::test_support::pipelines::PipelineBuilder;
    use crate::test_support::stubs::{StubJobs, StubRegistry, StubTails, dispatcher};

    fn app(id: &str) -> AppId {
        AppId::new(id)
    }

    #[tokio::test]
    async fn a_drain_returns_the_number_of_jobs_it_placed() {
        let pipeline = PipelineBuilder::for_project_id(ProjectId::new("p")).build();
        let jobs = Arc::new(StubJobs::with(vec![job(&pipeline), job(&pipeline)]));
        let registry = Arc::new(StubRegistry::accepting());
        registry.connect(&app("agent-1"));
        let scheduler = PendingJobScheduler::new(
            Arc::new(actions(Arc::new(RecordingPermissionService::new()))),
            dispatcher(registry.clone(), jobs, Arc::new(StubTails::default())),
        );

        assert_eq!(scheduler.drain(Some(vec![app("agent-2")])).await, 0);
        assert_eq!(scheduler.drain(None).await, 1);
        assert_eq!(scheduler.drain(None).await, 0, "one job per agent");
        assert_eq!(registry.dispatched_to(), [app("agent-1")]);
    }

    #[test]
    fn queued_wakes_merge_into_one_pass() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        for wake in [Some(app("b")), Some(app("a")), Some(app("b"))] {
            tx.send(wake).unwrap();
        }

        let targets = PendingJobScheduler::targets(Some(app("a")), &mut rx);

        assert_eq!(targets, Some(vec![app("a"), app("b")]));
        assert!(rx.try_recv().is_err(), "every queued wake is consumed");
    }

    #[test]
    fn a_wake_for_every_agent_wins() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tx.send(Some(app("b"))).unwrap();
        tx.send(None).unwrap();

        assert_eq!(PendingJobScheduler::targets(Some(app("a")), &mut rx), None);
        assert!(rx.try_recv().is_err(), "every queued wake is consumed");
        assert_eq!(PendingJobScheduler::targets(None, &mut rx), None);
    }
}
