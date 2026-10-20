//! The job log's actions through the engine, on stub ports.

use super::*;
use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::domain::caller::CallerContext;
use crate::domain::clock;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::ProjectId;
use crate::domain::ids::{AppId, JobId};
use crate::domain::job::JobLog;
use crate::domain::permission::Permission;
use crate::domain::pipeline::NodeId;
use crate::test_support::authz::{DenyingPermissionService, RecordingPermissionService, actions};
use crate::test_support::job_logs::{JobLogBuilder, job_log};
use crate::test_support::jobs::JobBuilder;
use crate::test_support::pipelines::{PipelineBuilder, node};
use crate::test_support::stubs::StubJobs;
use async_trait::async_trait;
use chrono::{Duration, Utc};
use futures_util::{StreamExt, stream};
use scylla_auth::authz::PermissionService;
use scylla_extension::Actions;
use std::sync::Mutex;

#[derive(Default)]
struct StubLogs {
    rows: Mutex<Vec<JobLog>>,
    reads: Mutex<Vec<&'static str>>,
}

fn page(items: Vec<JobLog>) -> PaginatedResult<JobLog> {
    let total = items.len() as u64;
    PaginatedResult::new(items, &PaginationParams::default(), total)
}

#[async_trait]
impl JobLogRepository for StubLogs {
    async fn create_many(&self, logs: &[JobLog]) -> DomainResult<()> {
        self.reads.lock().unwrap().push("insert");
        self.rows.lock().unwrap().extend_from_slice(logs);
        Ok(())
    }
    async fn list_by_job(
        &self,
        _: &JobId,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<JobLog>> {
        self.reads.lock().unwrap().push("job");
        Ok(page(self.rows.lock().unwrap().clone()))
    }
    async fn list_by_job_and_node(
        &self,
        _: &JobId,
        node_id: &NodeId,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<JobLog>> {
        self.reads.lock().unwrap().push("node");
        let rows = self.rows.lock().unwrap();
        Ok(page(
            rows.iter()
                .filter(|l| l.node_id() == node_id)
                .cloned()
                .collect(),
        ))
    }
    async fn list_all_by_job(&self, _: &JobId, _: Option<&NodeId>) -> DomainResult<Vec<JobLog>> {
        Ok(self.rows.lock().unwrap().clone())
    }
}

/// Replays `lines` to a subscriber, and records each line published.
#[derive(Default)]
struct StubLive {
    lines: Mutex<Vec<JobLog>>,
    subscribes: Mutex<usize>,
    published: Mutex<Vec<JobLog>>,
}

#[async_trait]
impl JobLogStreamPort for StubLive {
    fn open(&self, _: &JobId) {}
    fn publish(&self, log: &JobLog) {
        self.published.lock().unwrap().push(log.clone());
    }
    fn close(&self, _: &JobId) {}
    async fn subscribe(&self, _: &JobId, _: Option<&NodeId>) -> DomainResult<JobLogLiveStream> {
        *self.subscribes.lock().unwrap() += 1;
        let lines = self.lines.lock().unwrap().clone();
        Ok(Box::pin(stream::iter(lines.into_iter().map(Ok))))
    }
}

struct Lab {
    actions: Actions,
    uc: JobLogUseCases,
    logs: Arc<StubLogs>,
    live: Arc<StubLive>,
}

/// `job-1` runs `build` on `agent`; `test` has not started.
fn lab(permissions: Arc<dyn PermissionService>) -> Lab {
    let pipeline = PipelineBuilder::for_project_id(ProjectId::new("p"))
        .nodes(vec![node("build", &[]), node("test", &["build"])])
        .build();
    let job = JobBuilder::new(&pipeline)
        .id(JobId::new("job-1"))
        .running(true)
        .agent(AppId::new("agent"))
        .build()
        .apply_node_started(&NodeId::new("build").unwrap(), clock::now())
        .unwrap();
    let logs = Arc::new(StubLogs::default());
    let live = Arc::new(StubLive::default());
    Lab {
        actions: actions(permissions),
        uc: JobLogUseCases::new(
            logs.clone(),
            live.clone(),
            Arc::new(StubJobs::with(vec![job])),
        ),
        logs,
        live,
    }
}

fn list(node: Option<&str>) -> ListJobLogs {
    ListJobLogs {
        job_id: JobId::new("job-1"),
        node_id: node.map(|n| NodeId::new(n).unwrap()),
        pagination: None,
    }
}

fn agent() -> CallerContext {
    CallerContext::App(AppId::new("agent"))
}

fn append(job: &str, lines: &[&str]) -> AppendJobLogs {
    let job_id = JobId::new(job);
    AppendJobLogs {
        logs: lines
            .iter()
            .map(|line| job_log(&job_id, "build", line))
            .collect(),
        job_id,
    }
}

fn lines(logs: &[JobLog]) -> Vec<&str> {
    logs.iter().map(JobLog::line).collect()
}

#[tokio::test]
async fn an_append_checks_once_stores_one_batch_and_publishes_each_line_in_order() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());

    let stored = lab
        .actions
        .run(&lab.uc, &agent(), append("job-1", &["one", "two", "three"]))
        .await
        .unwrap();

    assert_eq!(lines(&stored), ["one", "two", "three"]);
    assert_eq!(*lab.logs.reads.lock().unwrap(), vec!["insert"]);
    assert_eq!(
        lines(&lab.live.published.lock().unwrap()),
        ["one", "two", "three"]
    );
    assert_eq!(
        permissions.permissions(),
        vec![Permission::AppendJobLog(JobId::new("job-1"))]
    );
}

#[tokio::test]
async fn only_the_agent_of_the_job_appends_to_it() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let other = CallerContext::App(AppId::new("other"));

    let err = lab
        .actions
        .run(&lab.uc, &other, append("job-1", &["hello"]))
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::BusinessRule(_)));
    assert!(lab.logs.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_line_of_another_job_refuses_the_batch() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let mut batch = append("job-1", &["mine"]);
    batch
        .logs
        .push(job_log(&JobId::new("job-2"), "build", "theirs"));

    let err = lab.actions.run(&lab.uc, &agent(), batch).await.unwrap_err();

    assert!(matches!(err, DomainError::Validation(_)));
    assert!(lab.logs.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_denied_append_stores_nothing() {
    let lab = lab(Arc::new(DenyingPermissionService::new()));

    let err = lab
        .actions
        .run(&lab.uc, &agent(), append("job-1", &["hello"]))
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.logs.rows.lock().unwrap().is_empty());
    assert!(lab.live.published.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_list_reads_every_node_or_the_one_it_names() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let job_id = JobId::new("job-1");
    lab.logs.rows.lock().unwrap().extend([
        job_log(&job_id, "build", "a"),
        job_log(&job_id, "test", "b"),
    ]);

    let all = lab
        .actions
        .run(&lab.uc, &agent(), list(None))
        .await
        .unwrap();
    let one = lab
        .actions
        .run(&lab.uc, &agent(), list(Some("build")))
        .await
        .unwrap();

    assert_eq!(all.items().len(), 2);
    assert_eq!(one.items().len(), 1);
    assert_eq!(*lab.logs.reads.lock().unwrap(), vec!["job", "node"]);
    assert_eq!(
        permissions.permissions(),
        vec![
            Permission::ReadJobLogs(job_id.clone()),
            Permission::ReadJobLogs(job_id.clone()),
            Permission::ReadJob(job_id),
        ]
    );
}

#[tokio::test]
async fn a_list_of_a_node_that_has_not_started_is_empty() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let job_id = JobId::new("job-1");
    lab.logs
        .rows
        .lock()
        .unwrap()
        .push(job_log(&job_id, "test", "early"));

    let page = lab
        .actions
        .run(&lab.uc, &agent(), list(Some("test")))
        .await
        .unwrap();

    assert!(page.items().is_empty());
    assert_eq!(page.metadata().total_count(), 0);
    assert!(lab.logs.reads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_tail_of_a_node_that_has_not_started_replays_nothing() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let job_id = JobId::new("job-1");
    let early = job_log(&job_id, "test", "early");
    let live = job_log(&job_id, "test", "live");
    lab.logs.rows.lock().unwrap().push(early);
    lab.live.lines.lock().unwrap().push(live);

    let tail = lab
        .actions
        .run(
            &lab.uc,
            &agent(),
            TailJobLogs {
                job_id,
                node_id: Some(NodeId::new("test").unwrap()),
            },
        )
        .await
        .unwrap();
    let lines: Vec<String> = tail.map(|l| l.unwrap().line().to_string()).collect().await;

    assert_eq!(lines, vec!["live"]);
}

#[tokio::test]
async fn a_tail_replays_the_snapshot_then_only_unseen_live_lines() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let job_id = JobId::new("job-1");
    let at = Utc::now();
    let stored = JobLogBuilder::new(&job_id, "build", "old")
        .timestamp(at)
        .build();
    let newer = JobLogBuilder::new(&job_id, "build", "new")
        .timestamp(at + Duration::seconds(1))
        .build();
    lab.logs.rows.lock().unwrap().push(stored.clone());
    lab.live
        .lines
        .lock()
        .unwrap()
        .extend([stored.clone(), newer.clone()]);

    let tail = lab
        .actions
        .run(
            &lab.uc,
            &agent(),
            TailJobLogs {
                job_id: job_id.clone(),
                node_id: None,
            },
        )
        .await
        .unwrap();
    let lines: Vec<String> = tail.map(|l| l.unwrap().line().to_string()).collect().await;

    assert_eq!(lines, vec!["old", "new"]);
    assert_eq!(
        permissions.permissions(),
        vec![Permission::ReadJobLogs(job_id)]
    );
}

#[tokio::test]
async fn a_denied_tail_never_subscribes() {
    let lab = lab(Arc::new(DenyingPermissionService::new()));

    let err = lab
        .actions
        .run(
            &lab.uc,
            &agent(),
            TailJobLogs {
                job_id: JobId::new("job-1"),
                node_id: None,
            },
        )
        .await
        .err()
        .unwrap();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(*lab.live.subscribes.lock().unwrap(), 0);
}
