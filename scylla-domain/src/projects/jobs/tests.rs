//! Unit tests of the state machine of a job.

mod approval;
mod checkout;

use time::OffsetDateTime;

use super::{Job, JobEvent, JobStatus, NodeRun, NodeStatus, RerunOrigin, StartJob};
use crate::identity::actors::ActorId;
use crate::integrations::sources::{JobSource, SourceEvent};
use crate::integrations::VcsProvider;
use crate::projects::pipeline_files::FileError;
use crate::projects::pipelines::{EnvValue, EnvVar, Node, NodeId, Runtime, Step};
use crate::DomainError;

/// Makes a node with one retry.
fn node(id: &str) -> Node {
    Node {
        id: NodeId::parse(id.to_owned()).unwrap(),
        needs: Vec::new(),
        step: Step::Exec(vec!["true".to_owned()]),
        env: Vec::new(),
        timeout_seconds: 60,
        retries: 1,
        working_dir: None,
        runtime: Runtime::Process,
    }
}

/// Makes a pending job of two nodes.
fn job() -> Job {
    let nodes = vec![node("build"), node("test")];
    Job {
        id: super::JobId::generate(),
        pipeline_id: crate::Id::generate(),
        project_id: crate::Id::generate(),
        workspace_id: crate::Id::generate(),
        created_by: ActorId::generate(),
        rerun_of: None,
        trigger_id: None,
        status: JobStatus::Pending,
        runs: nodes.iter().map(NodeRun::pending).collect(),
        nodes,
        agent_id: None,
        error: None,
        version: 1,
        created_at: OffsetDateTime::now_utc(),
        started_at: None,
        finished_at: None,
        source: None,
        approval: None,
        pipeline_file: None,
        file_errors: Vec::new(),
        checkout: None,
    }
}

/// Returns the id of a node.
fn id(name: &str) -> NodeId {
    NodeId::parse(name.to_owned()).unwrap()
}

/// A job runs its nodes, retries one time, and completes when each node ended.
#[test]
fn a_job_runs_to_completion() {
    // given
    let mut job = job();
    let now = OffsetDateTime::now_utc();
    let events = [
        JobEvent::JobStarted,
        JobEvent::NodeStarted(id("build")),
        JobEvent::NodeRetrying {
            node: id("build"),
            attempt: 2,
        },
        JobEvent::NodeCompleted {
            node: id("build"),
            exit_code: Some(0),
        },
        JobEvent::NodeStarted(id("test")),
        JobEvent::NodeCompleted {
            node: id("test"),
            exit_code: Some(0),
        },
    ];

    // when
    let applied: Result<Vec<_>, _> = events.iter().map(|event| job.apply(event, now)).collect();
    let early = job.clone().apply(&JobEvent::JobStarted, now);
    let completed = job.apply(&JobEvent::JobCompleted, now);

    // then
    assert!(applied.is_ok() && completed.is_ok());
    assert_eq!(early, Err(DomainError::NotAllowed));
    assert_eq!(job.status, JobStatus::Completed);
    assert_eq!(job.runs[0].attempt, 2);
}

/// An event that does not fit is refused: a third attempt, a node of a pending job, a
/// completion with a node that runs, an event after the end.
#[test]
fn an_event_must_fit_the_state() {
    // given
    let mut job = job();
    let now = OffsetDateTime::now_utc();
    let early = job.apply(&JobEvent::NodeStarted(id("build")), now);
    job.apply(&JobEvent::JobStarted, now).unwrap();
    job.apply(&JobEvent::NodeStarted(id("build")), now).unwrap();
    job.apply(
        &JobEvent::NodeRetrying {
            node: id("build"),
            attempt: 2,
        },
        now,
    )
    .unwrap();

    // when
    let third = job.apply(
        &JobEvent::NodeRetrying {
            node: id("build"),
            attempt: 3,
        },
        now,
    );
    let unknown = job.apply(&JobEvent::NodeStarted(id("deploy")), now);
    let completed = job.apply(&JobEvent::JobCompleted, now);
    job.apply(&JobEvent::JobFailed("disk full".to_owned()), now)
        .unwrap();
    let after = job.apply(&JobEvent::NodeStarted(id("test")), now);

    // then
    for result in [early, third, unknown, completed, after] {
        assert_eq!(result, Err(DomainError::NotAllowed));
    }
    assert_eq!(job.status, JobStatus::Failed);
    assert_eq!(job.error.as_deref(), Some("disk full"));
    assert!(job
        .runs
        .iter()
        .all(|run| run.status == NodeStatus::Cancelled));
}

/// The end of a job cancels the nodes that did not end, and a final job does not change.
#[test]
fn an_end_cancels_the_open_nodes() {
    // given
    let mut job = job();
    let now = OffsetDateTime::now_utc();
    job.apply(&JobEvent::JobStarted, now).unwrap();
    job.apply(&JobEvent::NodeStarted(id("build")), now).unwrap();
    job.apply(
        &JobEvent::NodeCompleted {
            node: id("build"),
            exit_code: Some(0),
        },
        now,
    )
    .unwrap();

    // when
    let cancelled = job.end(JobStatus::Cancelled, None, now);
    let again = job.end(JobStatus::Orphaned, None, now);

    // then
    assert!(cancelled && !again);
    assert_eq!(job.status, JobStatus::Cancelled);
    let states: Vec<_> = job.runs.iter().map(|run| run.status).collect();
    assert_eq!(states, vec![NodeStatus::Completed, NodeStatus::Cancelled]);
}

/// Makes the source of a push on `main`, from a fork if `fork` is set.
fn source(fork: Option<&str>) -> JobSource {
    JobSource {
        provider: VcsProvider::GitHub,
        repository: "octo/app".to_owned(),
        url: "https://github.com/octo/app".to_owned(),
        sha: "0123456789abcdef0123456789abcdef01234567".to_owned(),
        git_ref: Some("refs/heads/main".to_owned()),
        branch: Some("main".to_owned()),
        tag: None,
        pull_requests: Vec::new(),
        fork_repository: fork.map(str::to_owned),
        author_login: Some("octocat".to_owned()),
        avatar: None,
        message_title: "Fix the build".to_owned(),
        event: SourceEvent::Push,
        automatic: true,
        changes_pipeline_files: false,
    }
}

/// A job that waits for an approval is not final, and each state reads back from its name.
#[test]
fn a_job_that_waits_for_an_approval_is_not_final() {
    // given
    let all = [
        JobStatus::AwaitingApproval,
        JobStatus::Pending,
        JobStatus::Running,
        JobStatus::Completed,
        JobStatus::Failed,
        JobStatus::Cancelled,
        JobStatus::Orphaned,
    ];

    // when
    let open: Vec<_> = all
        .into_iter()
        .filter(|status| !status.is_final())
        .collect();

    // then
    assert_eq!(
        open,
        vec![
            JobStatus::AwaitingApproval,
            JobStatus::Pending,
            JobStatus::Running
        ]
    );
    for status in all {
        assert_eq!(JobStatus::parse(status.as_str()), Some(status));
    }
}

/// A start adds the variables that a node does not have. A job from a fork waits for an
/// approval, and a job of an invalid file fails at its creation.
#[test]
fn a_start_adds_the_variables_and_waits_for_a_fork() {
    // given
    let mut start = StartJob::rerun(job(), RerunOrigin::Person);
    start.nodes[0].env.push(EnvVar {
        name: "TARGET".to_owned(),
        value: EnvValue::Literal("own".to_owned()),
    });
    start.variables = vec![EnvVar {
        name: "TARGET".to_owned(),
        value: EnvValue::Literal("trigger".to_owned()),
    }];
    let actor = ActorId::generate();

    // when
    let plain = start.clone().into_new(actor);
    let fork = StartJob {
        source: Some(source(Some("evil/app"))),
        ..start.clone()
    }
    .into_new(actor);
    let invalid = StartJob {
        file_errors: vec![FileError::of_file(".scylla/pipeline.toml", "bad")],
        ..start
    }
    .into_new(actor);

    // then
    let values: Vec<_> = plain
        .nodes
        .iter()
        .map(|node| node.env[0].value.clone())
        .collect();
    assert_eq!(values[0], EnvValue::Literal("own".to_owned()));
    assert_eq!(values[1], EnvValue::Literal("trigger".to_owned()));
    assert_eq!(plain.status(), JobStatus::Pending);
    assert_eq!(fork.status(), JobStatus::AwaitingApproval);
    assert_eq!(invalid.status(), JobStatus::Failed);
}

/// A new run keeps the source of the job, and it is not automatic. A new run from the provider
/// has its event.
#[test]
fn a_rerun_keeps_the_source() {
    // given
    let mut old = job();
    old.source = Some(source(None));

    // when
    let person = StartJob::rerun(old.clone(), RerunOrigin::Person);
    let provider = StartJob::rerun(old.clone(), RerunOrigin::Provider);

    // then
    let kept = person.source.unwrap();
    assert_eq!(kept.sha, old.source.as_ref().unwrap().sha);
    assert_eq!((kept.event, kept.automatic), (SourceEvent::Push, false));
    assert_eq!(provider.source.unwrap().event, SourceEvent::ProviderRerun);
    assert_eq!(person.rerun_of, Some(old.id));
    assert_eq!(person.nodes, old.nodes);
}
