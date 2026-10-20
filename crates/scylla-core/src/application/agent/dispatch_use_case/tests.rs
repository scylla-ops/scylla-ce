use super::*;
use crate::application::agent::dispatch::DispatchNode;
use crate::domain::caller::{CallerContext, ServiceIdentity};
use crate::domain::errors::DomainResult;
use crate::domain::ids::{OrganizationId, ProjectId};
use crate::domain::job::JobStatus;
use crate::domain::pipeline::{Pipeline, PipelineNode};
use crate::test_support::authz::{RecordingPermissionService, actions};
use crate::test_support::jobs::{JobBuilder, job, stored};
use crate::test_support::pipelines::{PipelineBuilder, node};
use crate::test_support::stubs::{
    EchoResolver, ScopesByAgent, StubJobs, StubRegistry, StubTails, alice,
};
use async_trait::async_trait;
use std::collections::HashMap;

/// Resolves every node but the one named `broken`, as a secret deleted since the run.
struct BrokenSecret;

#[async_trait]
impl SecretResolver for BrokenSecret {
    async fn resolve(
        &self,
        project_id: &ProjectId,
        nodes: &[PipelineNode],
    ) -> DomainResult<Vec<DispatchNode>> {
        if nodes.iter().any(|n| n.id().as_str() == "broken") {
            return Err(DomainError::not_found("Secret", "gone"));
        }
        EchoResolver.resolve(project_id, nodes).await
    }
}

struct Lab {
    uc: DispatchUseCases,
    registry: Arc<StubRegistry>,
    jobs: Arc<StubJobs>,
    tails: Arc<StubTails>,
    scopes: Arc<ScopesByAgent>,
}

fn lab_with(
    registry: StubRegistry,
    jobs: Vec<Job>,
    scopes: ScopesByAgent,
    resolver: Arc<dyn SecretResolver>,
) -> Lab {
    let registry = Arc::new(registry);
    let jobs = Arc::new(StubJobs::with(jobs));
    let tails = Arc::new(StubTails::default());
    let scopes = Arc::new(scopes);
    Lab {
        uc: DispatchUseCases::new(
            registry.clone(),
            scopes.clone(),
            jobs.clone(),
            resolver,
            tails.clone(),
        ),
        registry,
        jobs,
        tails,
        scopes,
    }
}

fn lab(connected: &[&str], jobs: Vec<Job>) -> Lab {
    let registry = StubRegistry::accepting();
    for id in connected {
        registry.connect(&AppId::new(*id));
    }
    lab_with(
        registry,
        jobs,
        ScopesByAgent::everything(),
        Arc::new(EchoResolver),
    )
}

fn a_pipeline() -> Pipeline {
    PipelineBuilder::for_project_id(ProjectId::new("p")).build()
}

fn dispatcher() -> CallerContext {
    CallerContext::Service(ServiceIdentity::job_dispatcher())
}

async fn pass(lab: &Lab, agents: Option<Vec<AppId>>) -> DomainResult<Vec<Job>> {
    let permissions = Arc::new(RecordingPermissionService::new());
    let placed = actions(permissions.clone())
        .run(&lab.uc, &dispatcher(), DispatchPendingJobs { agents })
        .await;
    assert!(permissions.checks().is_empty(), "a pass asks no permission");
    placed
}

#[tokio::test]
async fn a_pass_places_the_oldest_job_on_each_idle_agent_that_may_run_it() {
    let pl = a_pipeline();
    let (first, second, third) = (job(&pl), job(&pl), job(&pl));
    let lab = lab(
        &["agent-1", "agent-2"],
        vec![first.clone(), second.clone(), third.clone()],
    );

    let placed = pass(&lab, None).await.unwrap();

    assert_eq!(placed.len(), 2);
    assert_eq!(
        lab.registry.dispatched_to(),
        [AppId::new("agent-1"), AppId::new("agent-2")]
    );
    assert_eq!(
        lab.registry.dispatched()[0].1.job_id,
        first.id().to_string()
    );
    assert_eq!(
        lab.registry.dispatched()[1].1.job_id,
        second.id().to_string()
    );
    assert!(lab.jobs.row(third.id()).agent_app_id().is_none());
    assert_eq!(lab.scopes.asked(), ["executeJob", "executeJob"]);
}

#[tokio::test]
async fn a_busy_agent_and_an_agent_that_sees_nothing_get_no_job() {
    let pl = a_pipeline();
    let busy = AppId::new("busy");
    let running = JobBuilder::new(&pl)
        .running(true)
        .agent(busy.clone())
        .build();
    let waiting = job(&pl);
    let registry = StubRegistry::accepting();
    registry.connect(&busy);
    registry.connect(&AppId::new("blind"));
    let lab = lab_with(
        registry,
        vec![running, waiting.clone()],
        ScopesByAgent::new(
            HashMap::from([(busy.clone(), Visibility::All)]),
            Visibility::none(),
        ),
        Arc::new(EchoResolver),
    );

    let placed = pass(&lab, None).await.unwrap();

    assert!(placed.is_empty());
    assert!(lab.registry.dispatched().is_empty());
    assert!(lab.jobs.row(waiting.id()).agent_app_id().is_none());
}

#[tokio::test]
async fn an_agent_sees_only_the_scopes_of_its_grants() {
    let pl = a_pipeline();
    let agent = AppId::new("agent-1");
    let registry = StubRegistry::accepting();
    registry.connect(&agent);
    let scoped = Visibility::Scoped {
        orgs: vec![OrganizationId::new("acme")],
        projects: vec![],
    };
    let lab = lab_with(
        registry,
        vec![job(&pl)],
        ScopesByAgent::new(HashMap::from([(agent.clone(), scoped)]), Visibility::none()),
        Arc::new(EchoResolver),
    );

    let placed = pass(&lab, None).await.unwrap();

    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0].agent_app_id(), Some(&agent));
}

#[tokio::test]
async fn a_targeted_pass_touches_only_the_listed_agents_that_are_connected() {
    let pl = a_pipeline();
    let lab = lab(&["agent-1", "agent-2"], vec![job(&pl), job(&pl)]);

    let placed = pass(&lab, Some(vec![AppId::new("agent-2"), AppId::new("gone")]))
        .await
        .unwrap();

    assert_eq!(placed.len(), 1);
    assert_eq!(lab.registry.dispatched_to(), [AppId::new("agent-2")]);
}

#[tokio::test]
async fn a_failed_send_returns_the_job_to_the_pool_and_wakes_the_dispatcher() {
    let pl = a_pipeline();
    let waiting = job(&pl);
    let registry = StubRegistry::closing();
    let stream = registry.connect(&AppId::new("agent-1"));
    let lab = lab_with(
        registry,
        vec![waiting.clone()],
        ScopesByAgent::everything(),
        Arc::new(EchoResolver),
    );

    let placed = pass(&lab, None).await.unwrap();

    assert!(placed.is_empty());
    assert_eq!(lab.jobs.released(), [stream]);
    let row = lab.jobs.row(waiting.id());
    assert!(row.agent_app_id().is_none());
    assert_eq!(row.status(), JobStatus::Pending);
    assert_eq!(lab.registry.wakes(), [None]);
}

#[tokio::test]
async fn a_job_is_placed_on_the_stream_of_the_pass_and_only_that_stream_releases_it() {
    let waiting = job(&a_pipeline());
    let lab = lab(&["agent-1"], vec![waiting.clone()]);
    let first = lab.registry.connected().remove(0);

    pass(&lab, None).await.unwrap();
    let newer = lab.registry.connect(&AppId::new("agent-1"));

    assert_eq!(lab.jobs.stream_of(waiting.id()), Some(first.id.clone()));
    assert_eq!(lab.uc.release(&newer).await.unwrap(), 0);
    assert_eq!(lab.uc.release(&first).await.unwrap(), 1);
    assert!(lab.jobs.row(waiting.id()).agent_app_id().is_none());
}

#[tokio::test]
async fn a_job_that_no_longer_dispatches_fails_and_the_agent_gets_the_next_one() {
    let broken = PipelineBuilder::for_project_id(ProjectId::new("p"))
        .nodes(vec![node("broken", &[])])
        .build();
    let (doomed, next) = (job(&broken), job(&a_pipeline()));
    let registry = StubRegistry::accepting();
    registry.connect(&AppId::new("agent-1"));
    let lab = lab_with(
        registry,
        vec![doomed.clone(), next.clone()],
        ScopesByAgent::everything(),
        Arc::new(BrokenSecret),
    );

    let placed = pass(&lab, None).await.unwrap();

    assert_eq!(lab.jobs.row(doomed.id()).status(), JobStatus::Failed);
    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0].id(), next.id());
    assert_eq!(lab.registry.dispatched()[0].1.job_id, next.id().to_string());
}

#[tokio::test]
async fn a_pass_refuses_a_caller_that_is_not_a_service() {
    let lab = lab(&["agent-1"], vec![job(&a_pipeline())]);

    let err = actions(Arc::new(RecordingPermissionService::new()))
        .run(&lab.uc, &alice(), DispatchPendingJobs { agents: None })
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.registry.dispatched().is_empty());
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
    assert!(lab.registry.cancelled().is_empty());

    lab.uc
        .recall(JobScope::Pipeline(pl.id()), async { Ok(()) })
        .await
        .unwrap();
    assert_eq!(lab.registry.cancelled(), [(agent, running.id().clone())]);
}
