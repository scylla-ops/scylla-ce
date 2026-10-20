use crate::application::job::{JobUseCases, ReapOrphanedJobs};
use crate::domain::caller::{CallerContext, ServiceIdentity};
use scylla_extension::Actions;
use std::sync::Arc;
use tracing::{instrument, warn};

/// Level-triggered: compares the live jobs with the open streams and the last contact of each
/// agent, so a restart or a reconnect needs no special path.
pub struct JobReaper {
    actions: Arc<Actions>,
    job_uc: Arc<JobUseCases>,
}

impl JobReaper {
    #[must_use]
    pub fn new(actions: Arc<Actions>, job_uc: Arc<JobUseCases>) -> Self {
        Self { actions, job_uc }
    }

    #[instrument(skip_all)]
    pub async fn reap(&self) -> u64 {
        let caller = CallerContext::Service(ServiceIdentity::job_reaper());
        match self
            .actions
            .run(&*self.job_uc, &caller, ReapOrphanedJobs)
            .await
        {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::AppId;
    use crate::domain::job::JobStatus;
    use crate::test_support::authz::{RecordingPermissionService, actions};
    use crate::test_support::jobs::JobBuilder;
    use crate::test_support::organizations::org;
    use crate::test_support::pipelines::pipeline;
    use crate::test_support::projects::project;
    use crate::test_support::stubs::{StubJobs, StubRegistry, StubTails, dispatcher};

    fn reaper(jobs: Arc<StubJobs>) -> JobReaper {
        let registry = Arc::new(StubRegistry::default());
        let tails = Arc::new(StubTails::default());
        JobReaper::new(
            Arc::new(actions(Arc::new(RecordingPermissionService::new()))),
            Arc::new(JobUseCases::new(
                jobs.clone(),
                tails.clone(),
                dispatcher(registry, jobs, tails),
            )),
        )
    }

    #[tokio::test]
    async fn a_reap_returns_the_number_of_jobs_it_changed() {
        let pl = pipeline(&project(&org("o"), "p"));
        let running = JobBuilder::new(&pl)
            .running(true)
            .agent(AppId::new("agent-gone"))
            .build();
        let jobs = Arc::new(StubJobs::with(vec![running]));
        let reaper = reaper(jobs.clone());

        assert_eq!(reaper.reap().await, 1);
        assert_eq!(reaper.reap().await, 0);
        assert_eq!(jobs.rows()[0].status(), JobStatus::Orphaned);
    }
}
