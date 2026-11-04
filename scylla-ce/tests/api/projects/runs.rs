//! The tests of the jobs on the stream of an agent: the placement with the role and the
//! capacity, the values of the secrets, the events, the logs, the containers and the cancel.

use std::time::Duration;

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::pipelines::PipelineHolder;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::agents::v1::open_stream_request::Message as Up;
use scylla_proto::agents::v1::open_stream_response::Message as Down;
use scylla_proto::agents::v1::{
    Dispatch, EventKind, Hello, JobEvent, LogBatch, OpenStreamRequest, OpenStreamResponse,
};
use scylla_proto::apps::v1::Host;
use scylla_proto::pipelines::v1::{env_var, Container, EnvVar, JobStatus, LogLine, LogStream};
use scylla_testkit::{exec_node, id, text, TestApp, TestKey};
use sqlx::PgPool;
use tokio::sync::mpsc::Sender;
use tokio::time::timeout;
use tonic::Streaming;

use crate::app;

/// An agent of a test, with its open stream.
pub(super) struct Agent {
    /// The machine of the agent.
    key: TestKey,
    /// The sender of the messages of the agent.
    up: Sender<OpenStreamRequest>,
    /// The messages of the server.
    down: Streaming<OpenStreamResponse>,
}

impl Agent {
    /// Reads the next message of the server, or `None` if none comes in `wait`.
    pub(super) async fn next(&mut self, wait: Duration) -> Option<Down> {
        let message = timeout(wait, self.down.message()).await.ok()?;
        message.expect("an open stream")?.message
    }

    /// Reads the next dispatch, in at most 10 seconds.
    pub(super) async fn dispatch(&mut self) -> Dispatch {
        loop {
            match self.next(Duration::from_secs(10)).await {
                Some(Down::Dispatch(dispatch)) => return dispatch,
                Some(_) => continue,
                None => panic!("no dispatch"),
            }
        }
    }

    /// Sends an event of a job.
    async fn event(&self, job: &str, kind: EventKind, node: &str) {
        let event = JobEvent {
            job_id: Some(id(job)),
            kind: kind.into(),
            node_id: node.to_owned(),
            attempt: 0,
            exit_code: Some(0),
            error: String::new(),
        };
        let message = OpenStreamRequest {
            message: Some(Up::Event(event)),
        };
        self.up.send(message).await.unwrap();
    }
}

/// Registers an agent in a workspace, with the role `runner` if `runner`, and opens its stream
/// with a capacity and a container runtime.
pub(super) async fn agent(
    app: &TestApp,
    owner: ActorId,
    workspace: WorkspaceId,
    runner: bool,
    capacity: u32,
    runtime: &str,
) -> Agent {
    let (key, token) = app.app_in(owner, workspace, true).await;
    if runner {
        app.grant(&[PipelineHolder::RUNNER.of(workspace, key.actor())])
            .await;
    }
    connect(app, key, &token, capacity, runtime).await
}

/// Opens the stream of a registered agent with a capacity and a container runtime, and waits for
/// the welcome.
pub(super) async fn connect(
    app: &TestApp,
    key: TestKey,
    token: &str,
    capacity: u32,
    runtime: &str,
) -> Agent {
    let (up, mut down) = app.open_stream(token).await.unwrap();
    let host = Host {
        version: "1.0.0".to_owned(),
        os: "linux".to_owned(),
        arch: "x86_64".to_owned(),
        hostname: "runner".to_owned(),
        cpu_count: 4,
        memory_bytes: 1 << 30,
        capacity,
        container_runtime: runtime.to_owned(),
    };
    let hello = OpenStreamRequest {
        message: Some(Up::Hello(Hello {
            host: Some(host),
            running_jobs: Vec::new(),
        })),
    };
    up.send(hello).await.unwrap();
    let welcome = timeout(Duration::from_secs(10), down.message())
        .await
        .unwrap();
    assert!(matches!(
        welcome.unwrap().unwrap().message,
        Some(Down::Welcome(_))
    ));
    Agent { key, up, down }
}

/// Creates a user, a project and a pipeline with these nodes. Returns the user, its workspace,
/// the project id and the pipeline id.
pub(super) async fn pipeline(
    app: &TestApp,
    nodes: Vec<scylla_proto::pipelines::v1::Node>,
) -> (ActorId, WorkspaceId, String, String) {
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let project = text(&project.id).to_owned();
    let pipeline = app.create_pipeline(owner, &project, nodes).await.unwrap();
    (owner, workspace, project, text(&pipeline.id).to_owned())
}

/// An agent with the role gets a job with the values of its secrets, runs it with events, and
/// its logs come back in a tail.
#[sqlx::test(migrations = false)]
async fn an_agent_runs_a_job_and_sends_its_logs(pool: PgPool) {
    // given
    let app = app(pool).await;
    let mut node = exec_node("build", &[]);
    node.env = vec![EnvVar {
        name: "TOKEN".to_owned(),
        value: Some(env_var::Value::Secret("API_TOKEN".to_owned())),
    }];
    let (owner, workspace, project, pipeline) = pipeline(&app, vec![node]).await;
    app.create_secret(owner, &project, "API_TOKEN", "s3cr3t")
        .await
        .unwrap();
    let mut agent = agent(&app, owner, workspace, true, 1, "").await;
    let job = app.run_pipeline(owner, &pipeline).await.unwrap();
    let job = text(&job.id).to_owned();

    // when
    let dispatch = agent.dispatch().await;
    agent.event(&job, EventKind::JobStarted, "").await;
    agent.event(&job, EventKind::NodeStarted, "build").await;
    let line = |text: &str| LogLine {
        stream: LogStream::Stdout.into(),
        text: text.to_owned(),
        at: None,
    };
    let logs = LogBatch {
        job_id: Some(id(job.clone())),
        node_id: "build".to_owned(),
        lines: vec![line("compiling"), line("done")],
    };
    agent
        .up
        .send(OpenStreamRequest {
            message: Some(Up::Logs(logs)),
        })
        .await
        .unwrap();
    agent.event(&job, EventKind::NodeCompleted, "build").await;
    agent.event(&job, EventKind::JobCompleted, "").await;
    let mut read = app.get_job(owner, &job).await.unwrap();
    for _ in 0..50 {
        if read.status == JobStatus::Completed as i32 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        read = app.get_job(owner, &job).await.unwrap();
    }
    let tail = app.tail_job_logs(owner, &job, "", "").await.unwrap();

    // then
    assert_eq!(text(&dispatch.job_id), job);
    let env = &dispatch.nodes[0].env[0];
    assert_eq!(
        (env.name.as_str(), env.value.as_str(), env.masked),
        ("TOKEN", "s3cr3t", true)
    );
    assert_eq!(read.status, JobStatus::Completed as i32);
    assert_eq!(text(&read.agent_id), agent.key.app);
    let lines: Vec<_> = tail
        .iter()
        .flat_map(|batch| &batch.lines)
        .map(|l| l.text.as_str())
        .collect();
    assert_eq!(lines, vec!["compiling", "done"]);
    let after = app
        .tail_job_logs(owner, &job, "", &tail[0].cursor)
        .await
        .unwrap();
    assert!(after.is_empty());
}

/// An agent with no role gets no job. An agent with a capacity of 1 gets the second job only
/// after the end of the first.
#[sqlx::test(migrations = false)]
async fn the_placement_follows_the_role_and_the_capacity(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, workspace, _, pipeline) = pipeline(&app, vec![exec_node("build", &[])]).await;
    let mut idle = agent(&app, owner, workspace, false, 4, "").await;
    let mut busy = agent(&app, owner, workspace, true, 1, "").await;
    let first = text(&app.run_pipeline(owner, &pipeline).await.unwrap().id).to_owned();
    let second = text(&app.run_pipeline(owner, &pipeline).await.unwrap().id).to_owned();

    // when
    let given = busy.dispatch().await;
    let waits = busy.next(Duration::from_millis(500)).await;
    busy.event(&first, EventKind::JobFailed, "").await;
    let next = busy.dispatch().await;

    // then
    assert_eq!(text(&given.job_id), first);
    assert!(waits.is_none());
    assert_eq!(text(&next.job_id), second);
    assert!(idle.next(Duration::from_millis(200)).await.is_none());
    let failed = app.get_job(owner, &first).await.unwrap();
    assert_eq!(failed.status, JobStatus::Failed as i32);
}

/// A job with a container goes only to an agent with a container runtime.
#[sqlx::test(migrations = false)]
async fn a_container_job_needs_a_container_runtime(pool: PgPool) {
    // given
    let app = app(pool).await;
    let mut node = exec_node("build", &[]);
    node.container = Some(Container {
        image: "rust:1.97".to_owned(),
    });
    let (owner, workspace, _, pipeline) = pipeline(&app, vec![node]).await;
    let mut plain = agent(&app, owner, workspace, true, 2, "").await;
    let mut docker = agent(&app, owner, workspace, true, 2, "docker").await;

    // when
    let job = text(&app.run_pipeline(owner, &pipeline).await.unwrap().id).to_owned();
    let dispatch = docker.dispatch().await;

    // then
    assert_eq!(text(&dispatch.job_id), job);
    assert_eq!(
        dispatch.nodes[0].container.as_ref().unwrap().image,
        "rust:1.97"
    );
    assert!(plain.next(Duration::from_millis(300)).await.is_none());
}

/// A cancel of a running job tells its agent, and a later event of the job gets a cancel too.
#[sqlx::test(migrations = false)]
async fn cancel_job_stops_the_job_on_its_agent(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, workspace, _, pipeline) = pipeline(&app, vec![exec_node("build", &[])]).await;
    let mut agent = agent(&app, owner, workspace, true, 1, "").await;
    let job = text(&app.run_pipeline(owner, &pipeline).await.unwrap().id).to_owned();
    agent.dispatch().await;
    agent.event(&job, EventKind::JobStarted, "").await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // when
    let cancelled = app.cancel_job(owner, &job).await.unwrap();
    let told = agent.next(Duration::from_secs(5)).await;
    agent.event(&job, EventKind::NodeStarted, "build").await;
    let again = agent.next(Duration::from_secs(5)).await;

    // then
    assert_eq!(cancelled.status, JobStatus::Cancelled as i32);
    for message in [told, again] {
        let Some(Down::Cancel(cancel)) = message else {
            panic!("a cancel, not {message:?}");
        };
        assert_eq!(text(&cancel.job_id), job);
    }
}
