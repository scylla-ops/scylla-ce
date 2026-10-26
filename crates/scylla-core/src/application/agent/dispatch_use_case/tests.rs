use super::*;
use crate::domain::errors::DomainError;
use crate::domain::ids::{AppId, ProjectId};
use crate::domain::job::JobStatus;
use crate::domain::pipeline::Pipeline;
use crate::test_support::jobs::{JobBuilder, job, stored};
use crate::test_support::pipelines::PipelineBuilder;
use crate::test_support::stubs::{StubJobs, StubRegistry, StubTails};

struct Lab {
    uc: DispatchUseCases,
    registry: Arc<StubRegistry>,
    tails: Arc<StubTails>,
}

fn lab(connected: &[&str], jobs: Vec<Job>) -> Lab {
    let registry = Arc::new(StubRegistry::accepting());
    for id in connected {
        registry.connect(&AppId::new(*id));
    }
    let tails = Arc::new(StubTails::default());
    Lab {
        uc: DispatchUseCases::new(
            registry.clone(),
            Arc::new(StubJobs::with(jobs)),
            tails.clone(),
        ),
        registry,
        tails,
    }
}

fn a_pipeline() -> Pipeline {
    PipelineBuilder::for_project_id(ProjectId::new("p")).build()
}

#[tokio::test]
async fn a_stop_cancels_the_job_on_its_agent_wakes_it_and_closes_the_tail() {
    let agent = AppId::new("agent-1");
    let placed = stored(&job(&a_pipeline()), Some(&agent));
    let lab = lab(&["agent-1"], vec![]);

    lab.uc.stop(&placed);
    lab.uc.stop(&job(&a_pipeline()));

    assert_eq!(
        lab.registry.cancelled(),
        [(agent.clone(), placed.id().clone())]
    );
    assert_eq!(lab.registry.wakes(), [Some(agent)]);
    assert_eq!(lab.tails.closed().len(), 2);
}

#[tokio::test]
async fn a_recall_stops_the_live_jobs_only_after_the_delete() {
    let pl = a_pipeline();
    let agent = AppId::new("agent-1");
    let running = stored(&JobBuilder::new(&pl).running(true).build(), Some(&agent));
    let done = JobBuilder::new(&pl)
        .terminated(JobStatus::Completed)
        .agent(agent.clone())
        .build();
    let lab = lab(&["agent-1"], vec![running.clone(), done]);

    let failed = lab
        .uc
        .recall(JobScope::Pipeline(pl.id()), async {
            Err::<(), _>(DomainError::stale("Pipeline", pl.id()))
        })
        .await;
    assert!(matches!(failed, Err(DomainError::Stale(_))));
    assert_eq!(lab.registry.cancelled(), Vec::<(AppId, JobId)>::new());

    lab.uc
        .recall(JobScope::Pipeline(pl.id()), async { Ok(()) })
        .await
        .unwrap();
    assert_eq!(lab.registry.cancelled(), [(agent, running.id().clone())]);
}
