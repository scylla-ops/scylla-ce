use crate::access::{CallerOf, Identity};
use crate::{Auth, AuthRpc, JobControl, JobControlRpc, Slice, Triggers, TriggersRpc};
use scylla_core::application::PendingJobScheduler;
use scylla_core::application::agent::DispatchPendingJobs;
use scylla_core::application::auth::PurgeExpiredSessions;
use scylla_core::application::job::ReapOrphanedJobs;
use scylla_core::application::trigger::{ClaimDueTriggers, ScheduleCronTriggers};
use scylla_domain::domain::caller::{CallerContext, ServiceIdentity};
use scylla_domain::domain::clock;
use scylla_domain::domain::ids::AppId;
use scylla_domain::domain::session::SessionClient;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use strata::{Contextual, RequestMeta};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

const CRON_CLAIM_BATCH: i64 = 100;

/// A background pass of the server. Each worker belongs to one slice. When the slice is on
/// strata, the worker runs on the edition, and the CE worker does not run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Worker {
    CronTriggers,
    DispatchJobs,
    ReapJobs,
    SweepSessions,
}

impl Worker {
    pub const ALL: &'static [Worker] = &[
        Worker::CronTriggers,
        Worker::DispatchJobs,
        Worker::ReapJobs,
        Worker::SweepSessions,
    ];

    #[must_use]
    pub fn slice(self) -> Slice {
        match self {
            Worker::CronTriggers => Slice::Triggers,
            Worker::DispatchJobs | Worker::ReapJobs => Slice::Jobs,
            Worker::SweepSessions => Slice::Auth,
        }
    }

    #[must_use]
    pub fn on_strata(self, switched: &[Slice]) -> bool {
        switched.contains(&self.slice())
    }
}

/// What the strata workers need from the server. `wakes` is the receiver of the dispatcher
/// wakes; give it when the jobs slice is on strata. A cancel of `stop` ends every worker.
#[derive(Default)]
pub struct Workers {
    pub wakes: Option<mpsc::UnboundedReceiver<Option<AppId>>>,
    pub stop: CancellationToken,
}

fn service<C: CallerOf>(identity: ServiceIdentity) -> C {
    C::from_parts(
        Identity::of(CallerContext::Service(identity)),
        SessionClient::default(),
        RequestMeta::NONE,
    )
}

/// One pass of the job reaper, as the `job_reaper` service.
pub async fn reap_jobs<E>(app: &E) -> u64
where
    E: JobControl,
    <E as Contextual>::Ctx: CallerOf,
{
    let ctx = service::<<E as Contextual>::Ctx>(ServiceIdentity::job_reaper());
    match JobControlRpc::reap_orphaned_jobs(&app.scoped(&ctx), ReapOrphanedJobs).await {
        Ok(0) => 0,
        Ok(n) => {
            warn!(jobs = n, "reaped the jobs of agents that are gone");
            n
        }
        Err(e) => {
            warn!(error = %e, "job reaper: reconciliation pass failed");
            0
        }
    }
}

/// One pass of the session sweeper, as the `session_sweeper` service.
pub async fn sweep_sessions<E>(app: &E) -> u64
where
    E: Auth,
    <E as Contextual>::Ctx: CallerOf,
{
    let ctx = service::<<E as Contextual>::Ctx>(ServiceIdentity::session_sweeper());
    match AuthRpc::purge_expired_sessions(&app.scoped(&ctx), PurgeExpiredSessions).await {
        Ok(0) => 0,
        Ok(n) => {
            info!(purged = n, "purged expired sessions");
            n
        }
        Err(e) => {
            warn!(error = %e, "session sweeper: purge pass failed");
            0
        }
    }
}

/// One pass of the cron scheduler, as the `cron_scheduler` service: seed, claim, then one fire
/// for each claimed trigger. A failed fire does not stop the pass. The return is the fires.
pub async fn fire_cron_triggers<E>(app: &E) -> usize
where
    E: Triggers,
    <E as Contextual>::Ctx: CallerOf,
{
    let ctx = service::<<E as Contextual>::Ctx>(ServiceIdentity::cron_scheduler());
    let view = app.scoped(&ctx);
    if let Err(e) = TriggersRpc::schedule_cron_triggers(&view, ScheduleCronTriggers).await {
        warn!(error = %e, "cron seed: could not seed unscheduled triggers");
    }
    let claim = ClaimDueTriggers {
        now: clock::now(),
        limit: CRON_CLAIM_BATCH,
    };
    let due = match TriggersRpc::claim_due_triggers(&view, claim).await {
        Ok(due) => due,
        Err(e) => {
            warn!(error = %e, "cron claim: could not claim due triggers");
            return 0;
        }
    };
    let mut fired = 0usize;
    for trigger in due {
        match Triggers::fire(app, trigger.id()).await {
            Ok(job) => {
                info!(trigger_id = %trigger.id(), job_id = %job.id(), "cron trigger fired");
                fired += 1;
            }
            Err(e) => {
                warn!(trigger_id = %trigger.id(), error = %e, "cron trigger fire failed");
            }
        }
    }
    if fired > 0 {
        info!(fired, "cron scheduler fired due triggers");
    }
    fired
}

/// One pass of the dispatcher, as the `job_dispatcher` service. `None` targets every agent.
/// The return is the number of jobs placed.
pub async fn dispatch_jobs<E>(app: &E, agents: Option<Vec<AppId>>) -> usize
where
    E: JobControl,
    <E as Contextual>::Ctx: CallerOf,
{
    let ctx = service::<<E as Contextual>::Ctx>(ServiceIdentity::job_dispatcher());
    let pass = DispatchPendingJobs { agents };
    match JobControlRpc::dispatch_pending_jobs(&app.scoped(&ctx), pass).await {
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

fn every<F, Fut>(stop: CancellationToken, period: Duration, mut pass: F)
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(period);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                () = stop.cancelled() => break,
                _ = tick.tick() => {}
            }
            pass().await;
        }
    });
}

/// Starts the worker of each slice in `slices` on `app`. The first tick of each worker is at
/// once. A wake names the agents of a dispatch pass, and the wakes that arrive during a pass
/// make one pass.
pub fn spawn<E>(app: &Arc<E>, slices: &[Slice], workers: Workers)
where
    E: JobControl + Auth + Triggers + Send + Sync + 'static,
    <E as Contextual>::Ctx: CallerOf,
{
    let Workers { mut wakes, stop } = workers;
    for worker in Worker::ALL.iter().filter(|w| w.on_strata(slices)) {
        let app = app.clone();
        match worker {
            Worker::CronTriggers => every(stop.clone(), Duration::from_secs(15), move || {
                let app = app.clone();
                async move {
                    fire_cron_triggers(&*app).await;
                }
            }),
            Worker::ReapJobs => every(stop.clone(), Duration::from_secs(30), move || {
                let app = app.clone();
                async move {
                    reap_jobs(&*app).await;
                }
            }),
            Worker::SweepSessions => every(stop.clone(), Duration::from_secs(3600), move || {
                let app = app.clone();
                async move {
                    sweep_sessions(&*app).await;
                }
            }),
            Worker::DispatchJobs => {
                let stop = stop.clone();
                let mut woken = wakes.take().unwrap_or_else(|| mpsc::unbounded_channel().1);
                tokio::spawn(async move {
                    let mut tick = tokio::time::interval(Duration::from_secs(30));
                    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
                    loop {
                        let first = tokio::select! {
                            () = stop.cancelled() => break,
                            Some(wake) = woken.recv() => wake,
                            _ = tick.tick() => None,
                        };
                        dispatch_jobs(&*app, PendingJobScheduler::targets(first, &mut woken)).await;
                    }
                });
            }
        }
        tracing::info!(worker = ?worker, edition = std::any::type_name::<E>(), "strata runs the worker");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_worker_follows_its_slice() {
        assert!(Worker::ALL.iter().all(|w| !w.on_strata(&[])));
        assert!(Worker::ALL.iter().all(|w| w.on_strata(Slice::ALL)));
        let jobs: Vec<Worker> = Worker::ALL
            .iter()
            .copied()
            .filter(|w| w.on_strata(&[Slice::Jobs]))
            .collect();
        assert_eq!(jobs, [Worker::DispatchJobs, Worker::ReapJobs]);
        assert!(Worker::SweepSessions.on_strata(&[Slice::Auth]));
        assert!(Worker::CronTriggers.on_strata(&[Slice::Triggers]));
    }
}
