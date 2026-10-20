//! The job's actions through the engine, on stub ports.

use super::*;
use crate::application::agent::AgentStream;
use crate::domain::caller::{CallerContext, ServiceIdentity};
use crate::domain::clock;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{AppId, JobId, OrganizationId, PipelineId, ProjectId, StreamId};
use crate::domain::job::{Job, JobStatus, NodeState};
use crate::domain::permission::Permission;
use crate::domain::pipeline::{NodeId, Pipeline};
use crate::test_support::authz::{
    DenyingPermissionService, RecordingPermissionService, actions, actions_with,
};
use crate::test_support::jobs::{JobBuilder, job, stored};
use crate::test_support::pipelines::{PipelineBuilder, node};
use crate::test_support::stubs::{StubJobs, StubRegistry, StubTails, alice, dispatcher};
use async_trait::async_trait;
use chrono::Duration;
use scylla_auth::authz::{PermissionService, Visibility};
use scylla_extension::{Actions, Gate, Hooks, Persist, Prepared};

struct Lab {
    actions: Actions,
    uc: JobUseCases,
    jobs: Arc<StubJobs>,
    registry: Arc<StubRegistry>,
    tails: Arc<StubTails>,
    /// The open stream of the agent.
    stream: AgentStream,
}

impl Lab {
    fn insert(&self, job: &Job) -> Job {
        self.jobs.insert(job)
    }

    async fn record(&self, job: &Job, event: JobEvent) -> DomainResult<Job> {
        self.record_at(job, event, clock::now()).await
    }

    async fn record_at(
        &self,
        job: &Job,
        event: JobEvent,
        at: chrono::DateTime<chrono::Utc>,
    ) -> DomainResult<Job> {
        let record = RecordJobStatus {
            job_id: job.id().clone(),
            event,
            at,
        };
        self.actions.run(&self.uc, &agent(), record).await
    }
}

fn lab(permissions: Arc<dyn PermissionService>) -> Lab {
    let jobs = Arc::new(StubJobs::default());
    let registry = Arc::new(StubRegistry::default());
    let stream = registry.connect(&agent_id());
    let tails = Arc::new(StubTails::default());
    Lab {
        actions: actions(permissions),
        uc: JobUseCases::new(
            jobs.clone(),
            tails.clone(),
            dispatcher(registry.clone(), jobs.clone(), tails.clone()),
        ),
        jobs,
        registry,
        tails,
        stream,
    }
}

fn agent_id() -> AppId {
    AppId::new("agent")
}

fn agent() -> CallerContext {
    CallerContext::App(agent_id())
}

fn two_nodes() -> Pipeline {
    PipelineBuilder::for_project_id(ProjectId::new("p"))
        .nodes(vec![node("a", &[]), node("b", &["a"])])
        .build()
}

/// A pending job placed on the agent.
fn placed(pipeline: &Pipeline) -> Job {
    JobBuilder::new(pipeline).agent(agent_id()).build()
}

/// A running job placed on the agent, with node `a` running.
fn busy(pipeline: &Pipeline) -> Job {
    placed(pipeline)
        .start(clock::now())
        .unwrap()
        .apply_node_started(&NodeId::new("a").unwrap(), clock::now())
        .unwrap()
}

#[tokio::test]
async fn a_status_event_of_the_agent_checks_write_job_status_and_persists_the_transition() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let seeded = lab.insert(&placed(&two_nodes()));

    let job = lab.record(&seeded, JobEvent::JobStarted).await.unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::WriteJobStatus(seeded.id().clone())]
    );
    assert_eq!(job.status(), JobStatus::Running);
    assert_eq!(lab.jobs.row(seeded.id()).status(), JobStatus::Running);
    assert_eq!(lab.tails.opened(), [seeded.id().clone()]);
}

#[tokio::test]
async fn a_denied_status_event_writes_nothing() {
    let lab = lab(Arc::new(DenyingPermissionService::new()));
    let seeded = lab.insert(&placed(&two_nodes()));

    let err = lab.record(&seeded, JobEvent::JobStarted).await.unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(lab.jobs.row(seeded.id()).status(), JobStatus::Pending);
}

#[tokio::test]
async fn only_the_agent_of_a_live_job_reports_it() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let pl = two_nodes();
    let elsewhere = lab.insert(&JobBuilder::new(&pl).agent(AppId::new("other")).build());
    let unplaced = lab.insert(&job(&pl));
    let ended = lab.insert(
        &JobBuilder::new(&pl)
            .terminated(JobStatus::Orphaned)
            .agent(agent_id())
            .build(),
    );

    for job in [&elsewhere, &unplaced, &ended] {
        let err = lab.record(job, JobEvent::JobStarted).await.unwrap_err();
        assert!(matches!(err, DomainError::BusinessRule(_)), "{err}");
    }
    let err = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            RecordJobStatus {
                job_id: elsewhere.id().clone(),
                event: JobEvent::JobStarted,
                at: clock::now(),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(lab.jobs.row(ended.id()).status(), JobStatus::Orphaned);
}

#[tokio::test]
async fn the_time_of_an_event_stays_between_the_creation_and_now() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let seeded = lab.insert(&placed(&two_nodes()));

    let early = lab
        .record_at(
            &seeded,
            JobEvent::JobStarted,
            seeded.created_at() - Duration::hours(1),
        )
        .await
        .unwrap();
    let late = lab
        .record_at(
            &early,
            JobEvent::JobFailed {
                error: "boom".into(),
            },
            clock::now() + Duration::hours(1),
        )
        .await
        .unwrap();

    assert_eq!(early.started_at(), Some(seeded.created_at()));
    assert!(late.finished_at().unwrap() <= clock::now());
}

#[tokio::test]
async fn an_ended_job_closes_its_tail_and_frees_its_agent() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let seeded = lab.insert(&busy(&two_nodes()));

    let job = lab.record(&seeded, JobEvent::JobCompleted).await.unwrap();

    assert_eq!(job.status(), JobStatus::Completed);
    assert_eq!(
        job.find_execution(&NodeId::new("a").unwrap())
            .unwrap()
            .state(),
        NodeState::Cancelled
    );
    assert_eq!(lab.tails.closed(), [seeded.id().clone()]);
    assert_eq!(lab.registry.wakes(), [Some(agent_id())]);
}

#[tokio::test]
async fn a_cancel_ends_a_live_job_and_stops_it_on_its_agent() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let pl = two_nodes();
    let running = lab.insert(&busy(&pl));
    let pending = lab.insert(&job(&pl));

    let cancelled = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            CancelJob {
                id: running.id().clone(),
            },
        )
        .await
        .unwrap();
    let never_placed = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            CancelJob {
                id: pending.id().clone(),
            },
        )
        .await
        .unwrap();

    assert_eq!(cancelled.status(), JobStatus::Cancelled);
    assert_eq!(never_placed.status(), JobStatus::Cancelled);
    for node in cancelled.node_executions() {
        assert_eq!(node.state(), NodeState::Cancelled);
    }
    assert_eq!(
        lab.registry.cancelled(),
        [(agent_id(), running.id().clone())]
    );
    assert_eq!(lab.registry.wakes(), [Some(agent_id())]);
    assert_eq!(lab.tails.closed().len(), 2);
    assert_eq!(
        permissions.permissions(),
        vec![
            Permission::UpdateJob(running.id().clone()),
            Permission::UpdateJob(pending.id().clone()),
        ]
    );
}

#[tokio::test]
async fn a_cancel_of_an_ended_job_fails_its_precondition() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let ended = lab.insert(
        &JobBuilder::new(&two_nodes())
            .terminated(JobStatus::Completed)
            .build(),
    );

    let err = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            CancelJob {
                id: ended.id().clone(),
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::BusinessRule(_)));
    assert!(lab.registry.cancelled().is_empty());
}

#[tokio::test]
async fn a_delete_returns_the_tombstone_and_stops_a_live_job() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let pl = two_nodes();
    let running = lab.insert(&busy(&pl));
    let ended = lab.insert(
        &JobBuilder::new(&pl)
            .terminated(JobStatus::Failed)
            .agent(agent_id())
            .build(),
    );

    let deleted = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            DeleteJob {
                id: running.id().clone(),
            },
        )
        .await
        .unwrap();
    lab.actions
        .run(
            &lab.uc,
            &alice(),
            DeleteJob {
                id: ended.id().clone(),
            },
        )
        .await
        .unwrap();

    assert_eq!(deleted.last_state().id(), running.id());
    assert!(lab.jobs.rows().is_empty());
    assert_eq!(
        lab.registry.cancelled(),
        [(agent_id(), running.id().clone())]
    );
    assert_eq!(
        permissions.permissions()[0],
        Permission::DeleteJob(running.id().clone())
    );
}

#[tokio::test]
async fn a_delete_of_a_missing_job_is_not_found_and_stops_nothing() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));

    let err = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            DeleteJob {
                id: JobId::new("gone"),
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::NotFound(_)));
    assert!(lab.registry.cancelled().is_empty());
}

#[tokio::test]
async fn each_read_checks_its_own_permission() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let seeded = lab.insert(&job(&two_nodes()));
    let caller = alice();

    let got = lab
        .actions
        .run(
            &lab.uc,
            &caller,
            GetJob {
                id: seeded.id().clone(),
            },
        )
        .await
        .unwrap();
    lab.actions
        .run(&lab.uc, &caller, ListJobs { pagination: None })
        .await
        .unwrap();
    lab.actions
        .run(
            &lab.uc,
            &caller,
            ListPipelineJobs {
                pipeline_id: PipelineId::new("pipe"),
                pagination: None,
            },
        )
        .await
        .unwrap();
    lab.actions
        .run(
            &lab.uc,
            &caller,
            ListProjectJobs {
                project_id: ProjectId::new("p"),
                pagination: None,
            },
        )
        .await
        .unwrap();
    lab.actions
        .run(
            &lab.uc,
            &caller,
            ListOrganizationJobs {
                organization_id: OrganizationId::new("acme"),
                agent_id: None,
                pagination: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(got.id(), seeded.id());
    assert_eq!(
        permissions.permissions(),
        vec![
            Permission::ReadJob(seeded.id().clone()),
            Permission::ListJobs,
            Permission::ListJobsByPipeline(PipelineId::new("pipe")),
            Permission::ListJobsByProject(ProjectId::new("p")),
            Permission::ListJobsByOrganization(OrganizationId::new("acme")),
        ]
    );
}

#[tokio::test]
async fn an_organization_list_with_an_agent_keeps_its_permission_and_lists_only_that_agent() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let pipeline = two_nodes();
    let on_agent = lab.insert(&placed(&pipeline));
    let ended = lab.insert(
        &JobBuilder::new(&pipeline)
            .terminated(JobStatus::Completed)
            .agent(agent_id())
            .build(),
    );
    lab.insert(
        &JobBuilder::new(&pipeline)
            .agent(AppId::new("other"))
            .build(),
    );
    lab.insert(&job(&pipeline));

    let page = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            ListOrganizationJobs {
                organization_id: OrganizationId::new("acme"),
                agent_id: Some(agent_id()),
                pagination: None,
            },
        )
        .await
        .unwrap();

    let ids: Vec<&JobId> = page.items().iter().map(Job::id).collect();
    assert_eq!(ids, [on_agent.id(), ended.id()]);
    assert_eq!(
        permissions.permissions(),
        [Permission::ListJobsByOrganization(OrganizationId::new(
            "acme"
        ))]
    );
}

fn reaper() -> CallerContext {
    CallerContext::Service(ServiceIdentity::job_reaper())
}

/// A stream of `agent` that the registry does not hold.
fn another_stream(agent: &AppId) -> AgentStream {
    AgentStream {
        agent: agent.clone(),
        id: StreamId::generate(),
    }
}

impl Lab {
    async fn reap(&self) -> DomainResult<u64> {
        self.actions
            .run(&self.uc, &reaper(), ReapOrphanedJobs)
            .await
    }

    async fn release(&self, stream: &AgentStream) -> DomainResult<u64> {
        let release = ReleaseAgentJobs {
            stream: stream.id.clone(),
        };
        self.actions.run(&self.uc, &agent(), release).await
    }

    async fn reconcile(&self, running: &[&Job]) -> DomainResult<u64> {
        let reconcile = ReconcileAgentJobs {
            running: running.iter().map(|job| job.id().clone()).collect(),
        };
        self.actions.run(&self.uc, &agent(), reconcile).await
    }
}

#[tokio::test]
async fn a_reap_pass_ends_the_running_jobs_of_gone_agents_and_frees_their_streams() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let pl = two_nodes();
    let gone = AppId::new("gone");
    let lost = lab.insert(&stored(&busy(&pl), Some(&gone)));
    let unstarted = lab.jobs.place(&job(&pl), &another_stream(&gone));
    let kept = lab.insert(&busy(&pl));

    let changed = lab.reap().await.unwrap();

    assert_eq!(changed, 2);
    let orphaned = lab.jobs.row(lost.id());
    assert_eq!(orphaned.status(), JobStatus::Orphaned);
    for node in orphaned.node_executions() {
        assert_eq!(node.state(), NodeState::Cancelled);
    }
    assert!(lab.jobs.row(unstarted.id()).agent_app_id().is_none());
    assert_eq!(lab.jobs.row(kept.id()).status(), JobStatus::Running);
    assert_eq!(lab.tails.closed(), [lost.id().clone()]);
    assert!(lab.registry.wakes().contains(&None));
    assert!(permissions.permissions().is_empty());
}

#[tokio::test]
async fn a_reap_pass_frees_a_stream_that_is_gone_while_its_agent_is_back() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let pl = two_nodes();
    let earlier = lab.jobs.place(&job(&pl), &another_stream(&agent_id()));
    let current = lab.jobs.place(&job(&pl), &lab.stream);

    assert_eq!(lab.reap().await.unwrap(), 1);

    assert!(lab.jobs.row(earlier.id()).agent_app_id().is_none());
    assert_eq!(lab.jobs.row(current.id()).agent_app_id(), Some(&agent_id()));
}

/// The agent's report lands between the read of a pass and its write.
struct ReportLandsFirst {
    jobs: Arc<StubJobs>,
    id: JobId,
}

#[async_trait]
impl Gate<Persist<ReapOrphanedJobs>> for ReportLandsFirst {
    async fn check(&self, _: &Prepared<ReapOrphanedJobs>) -> DomainResult<()> {
        self.jobs.touch(&self.id);
        Ok(())
    }
}

#[tokio::test]
async fn a_reap_pass_skips_a_job_that_changed_since_it_read_it() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let lost = lab.insert(&stored(&busy(&two_nodes()), Some(&AppId::new("gone"))));
    let mut hooks = Hooks::new();
    hooks.gate::<Persist<ReapOrphanedJobs>>(Arc::new(ReportLandsFirst {
        jobs: lab.jobs.clone(),
        id: lost.id().clone(),
    }));

    let changed = actions_with(Arc::new(RecordingPermissionService::new()), hooks)
        .run(&lab.uc, &reaper(), ReapOrphanedJobs)
        .await
        .unwrap();

    assert_eq!(changed, 0);
    assert_eq!(lab.jobs.row(lost.id()).status(), JobStatus::Running);
    assert!(lab.tails.closed().is_empty());
}

/// Between the read of a pass and its write, the stream that the pass found gone lets its job
/// go, and the new stream of the agent takes it.
struct AgentComesBack {
    jobs: Arc<StubJobs>,
    gone: AgentStream,
    back: AgentStream,
}

#[async_trait]
impl Gate<Persist<ReapOrphanedJobs>> for AgentComesBack {
    async fn check(&self, _: &Prepared<ReapOrphanedJobs>) -> DomainResult<()> {
        self.jobs.release(&self.gone).await?;
        self.jobs.claim_next(&self.back, &Visibility::All).await?;
        Ok(())
    }
}

#[tokio::test]
async fn a_reap_pass_leaves_the_job_that_a_new_stream_took_since_it_read() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let returning = AppId::new("returning");
    let gone = another_stream(&returning);
    let waiting = lab.jobs.place(&job(&two_nodes()), &gone);
    let back = another_stream(&returning);
    let mut hooks = Hooks::new();
    hooks.gate::<Persist<ReapOrphanedJobs>>(Arc::new(AgentComesBack {
        jobs: lab.jobs.clone(),
        gone: gone.clone(),
        back: back.clone(),
    }));

    let changed = actions_with(Arc::new(RecordingPermissionService::new()), hooks)
        .run(&lab.uc, &reaper(), ReapOrphanedJobs)
        .await
        .unwrap();

    assert_eq!(changed, 0);
    assert_eq!(lab.jobs.row(waiting.id()).agent_app_id(), Some(&returning));
    assert_eq!(lab.jobs.stream_of(waiting.id()), Some(back.id));
}

#[tokio::test]
async fn a_reap_pass_refuses_a_caller_that_is_not_a_service() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    lab.jobs
        .place(&job(&two_nodes()), &another_stream(&agent_id()));

    let err = lab
        .actions
        .run(&lab.uc, &alice(), ReapOrphanedJobs)
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.jobs.released().is_empty());
}

#[tokio::test]
async fn the_end_of_a_stream_returns_only_its_own_jobs_to_the_pool() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let pl = two_nodes();
    let earlier = another_stream(&agent_id());
    let left = lab.jobs.place(&job(&pl), &earlier);
    let taken = lab.jobs.place(&job(&pl), &lab.stream);
    let running = lab.insert(&busy(&pl));

    let released = lab.release(&earlier).await.unwrap();
    let again = lab.release(&earlier).await.unwrap();

    assert_eq!((released, again), (1, 0));
    assert!(lab.jobs.row(left.id()).agent_app_id().is_none());
    assert_eq!(lab.jobs.row(taken.id()).agent_app_id(), Some(&agent_id()));
    assert_eq!(lab.jobs.stream_of(taken.id()), Some(lab.stream.id.clone()));
    assert_eq!(lab.jobs.row(running.id()).agent_app_id(), Some(&agent_id()));
    assert_eq!(lab.registry.wakes(), [None]);
}

#[tokio::test]
async fn only_an_agent_releases_or_reconciles_its_jobs() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));

    let release = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            ReleaseAgentJobs {
                stream: lab.stream.id.clone(),
            },
        )
        .await;
    let reconcile = lab
        .actions
        .run(&lab.uc, &alice(), ReconcileAgentJobs { running: vec![] })
        .await;

    assert!(matches!(release, Err(DomainError::Forbidden(_))));
    assert!(matches!(reconcile, Err(DomainError::Forbidden(_))));
    assert!(lab.jobs.released().is_empty());
}

#[tokio::test]
async fn a_reconcile_trusts_the_list_of_the_agent() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let pl = two_nodes();
    let lost = lab.insert(&busy(&pl));
    let kept = lab.insert(&busy(&pl));
    let unstarted = lab.jobs.place(&job(&pl), &another_stream(&agent_id()));
    let elsewhere = lab.insert(&JobBuilder::new(&pl).agent(AppId::new("other")).build());
    let deleted = job(&pl);

    let changed = lab
        .reconcile(&[&kept, &unstarted, &elsewhere, &deleted])
        .await
        .unwrap();

    assert_eq!(changed, 1);
    assert_eq!(lab.jobs.row(lost.id()).status(), JobStatus::Orphaned);
    assert_eq!(lab.jobs.row(kept.id()).status(), JobStatus::Running);
    assert_eq!(lab.jobs.row(unstarted.id()).status(), JobStatus::Pending);
    assert_eq!(lab.tails.opened(), [kept.id().clone()]);
    let cancelled: Vec<JobId> = lab
        .registry
        .cancelled()
        .into_iter()
        .map(|(_, id)| id)
        .collect();
    assert_eq!(
        cancelled,
        [
            lost.id().clone(),
            unstarted.id().clone(),
            elsewhere.id().clone(),
            deleted.id().clone(),
        ]
    );
}

#[tokio::test]
async fn a_hello_leaves_the_job_placed_on_the_open_stream() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let taken = lab.jobs.place(&job(&two_nodes()), &lab.stream);

    let changed = lab.reconcile(&[]).await.unwrap();

    assert_eq!(changed, 0);
    assert_eq!(lab.jobs.row(taken.id()).agent_app_id(), Some(&agent_id()));
    assert!(lab.registry.cancelled().is_empty());
    assert!(lab.jobs.released().is_empty());
}
