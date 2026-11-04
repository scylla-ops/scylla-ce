//! The tests of the agent against the real server of the Community edition: the join, the run of
//! a pipeline with its secrets, its retries, its failure and its logs, a cancel, the name of its
//! key, and the join at a first start.

use std::path::PathBuf;
use std::time::Duration;

use scylla_agent::{AgentConfig, Settings};
use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::pipelines::PipelineHolder;
use scylla_proto::pipelines::v1::{
    env_var, node, EnvVar, Exec, Job, JobStatus, Node, NodeStatus, Script,
};
use scylla_testkit::{exec_node, text, TestApp};
use sqlx::PgPool;
use tokio::sync::oneshot;

/// Starts the Community edition.
async fn app(pool: PgPool) -> TestApp {
    TestApp::spawn(pool, scylla_ce::edition()).await
}

/// A running agent of a test, and how to stop it.
struct Running {
    /// The folder of the work folders.
    root: PathBuf,
    /// Stops the agent.
    stop: Option<oneshot::Sender<()>>,
    /// The task of the agent.
    task: tokio::task::JoinHandle<Result<(), scylla_agent::AgentError>>,
}

impl Running {
    /// Stops the agent, and deletes its folders.
    async fn stop(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let _ = self.task.await;
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Joins a new agent in the workspace of the owner, gives it the role runner, and starts it.
async fn agent(app: &TestApp, owner: ActorId) -> Running {
    let workspace = app.workspace_of(owner).await;
    let created = app.create_app(owner, workspace, true).await.unwrap();
    let url = format!("http://{}", app.addr);
    let config: AgentConfig = scylla_agent::join(&url, &created.join_code).await.unwrap();
    let id = ActorId::parse(&config.app_id).unwrap();
    app.grant(&[PipelineHolder::RUNNER.of(workspace, id)]).await;
    let root = std::env::temp_dir().join(format!("scylla-agent-{}", ActorId::generate()));
    let settings = Settings {
        workspace_root: root.clone(),
        capacity: 2,
        container_runtime: None,
        keep_workspace: false,
    };
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(scylla_agent::run(config, settings, async {
        let _ = stopped.await;
    }));
    Running {
        root,
        stop: Some(stop),
        task,
    }
}

/// Reads a job again until `done` says yes, for at most 30 seconds.
async fn wait(app: &TestApp, owner: ActorId, job: &str, done: impl Fn(&Job) -> bool) -> Job {
    for _ in 0..300 {
        let read = app.get_job(owner, job).await.unwrap();
        if done(&read) {
            return read;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the job did not reach its state");
}

/// Makes a node that runs a shell command.
fn shell(node_id: &str, needs: &[&str], command: &str) -> Node {
    let mut node = exec_node(node_id, needs);
    node.step = Some(node::Step::Exec(Exec {
        argv: vec!["sh".to_owned(), "-c".to_owned(), command.to_owned()],
    }));
    node
}

/// The agent runs the nodes in the order of their needs, masks the secrets in the logs, retries
/// a failed node, skips the nodes after a failure, and fails the job.
#[sqlx::test(migrations = false)]
async fn the_agent_runs_a_pipeline(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let project = text(&app.create_project(owner, "alpha").await.unwrap().id).to_owned();
    app.create_secret(owner, &project, "API_TOKEN", "s3cr3t")
        .await
        .unwrap();
    let mut build = exec_node("build", &[]);
    build.step = Some(node::Step::Script(Script {
        shell: 0,
        source: "echo hello $TOKEN\necho done >&2\n".to_owned(),
    }));
    build.env = vec![EnvVar {
        name: "TOKEN".to_owned(),
        value: Some(env_var::Value::Secret("API_TOKEN".to_owned())),
    }];
    let mut test = shell("test", &["build"], "echo try; exit 3");
    test.retries = 1;
    let deploy = shell("deploy", &["test"], "true");
    let pipeline = app
        .create_pipeline(owner, &project, vec![build, test, deploy])
        .await
        .unwrap();
    let running = agent(&app, owner).await;

    // when
    let job = app.run_pipeline(owner, text(&pipeline.id)).await.unwrap();
    let id = text(&job.id).to_owned();
    let ended = wait(&app, owner, &id, |job| {
        job.status == JobStatus::Failed as i32
    })
    .await;
    let logs = app.tail_job_logs(owner, &id, "build", "").await.unwrap();
    running.stop().await;

    // then
    let states: Vec<_> = ended
        .runs
        .iter()
        .map(|run| (run.node_id.as_str(), run.status, run.attempt))
        .collect();
    assert_eq!(
        states,
        vec![
            ("build", NodeStatus::Completed as i32, 1),
            ("test", NodeStatus::Failed as i32, 2),
            ("deploy", NodeStatus::Skipped as i32, 0),
        ]
    );
    assert_eq!(ended.runs[1].exit_code, Some(3));
    assert!(ended.error.contains("test"), "{}", ended.error);
    let lines: Vec<_> = logs
        .iter()
        .flat_map(|batch| &batch.lines)
        .map(|line| line.text.as_str())
        .collect();
    assert!(lines.contains(&"hello ***"), "{lines:?}");
    assert!(lines.contains(&"done"), "{lines:?}");
}

/// A process that does not start writes nothing, so the agent writes the reason in the logs of
/// the node.
#[sqlx::test(migrations = false)]
async fn the_agent_reports_a_node_that_did_not_start(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let project = text(&app.create_project(owner, "alpha").await.unwrap().id).to_owned();
    let mut missing = exec_node("missing", &[]);
    missing.step = Some(node::Step::Exec(Exec {
        argv: vec!["scylla-no-such-program".to_owned()],
    }));
    let pipeline = app
        .create_pipeline(owner, &project, vec![missing])
        .await
        .unwrap();
    let running = agent(&app, owner).await;

    // when
    let job = app.run_pipeline(owner, text(&pipeline.id)).await.unwrap();
    let id = text(&job.id).to_owned();
    wait(&app, owner, &id, |job| {
        job.status == JobStatus::Failed as i32
    })
    .await;
    let logs = app.tail_job_logs(owner, &id, "missing", "").await.unwrap();
    running.stop().await;

    // then
    let lines: Vec<_> = logs
        .iter()
        .flat_map(|batch| &batch.lines)
        .map(|line| line.text.as_str())
        .collect();
    assert!(
        lines
            .iter()
            .any(|line| line
                .starts_with("scylla-agent: attempt 1 failed: the agent did not find the program scylla-no-such-program")),
        "{lines:?}"
    );
}

/// A cancel stops the running node of a job with its child processes, and the agent deletes the
/// work folder.
#[sqlx::test(migrations = false)]
async fn the_agent_stops_a_cancelled_job(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let project = text(&app.create_project(owner, "alpha").await.unwrap().id).to_owned();
    let nodes = vec![shell("wait", &[], "sleep 30 & sleep 30")];
    let pipeline = app.create_pipeline(owner, &project, nodes).await.unwrap();
    let running = agent(&app, owner).await;
    let job = app.run_pipeline(owner, text(&pipeline.id)).await.unwrap();
    let id = text(&job.id).to_owned();
    wait(&app, owner, &id, |job| {
        job.runs[0].status == NodeStatus::Running as i32
    })
    .await;
    let folder = running.root.join(&id);

    // when
    app.cancel_job(owner, &id).await.unwrap();
    let mut gone = false;
    for _ in 0..100 {
        if !folder.exists() {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    running.stop().await;

    // then
    assert!(gone, "the work folder of a cancelled job stays");
    let read = app.get_job(owner, &id).await.unwrap();
    assert_eq!(read.status, JobStatus::Cancelled as i32);
}

/// The join gives the key of the agent the name of its machine.
#[sqlx::test(migrations = false)]
async fn the_join_names_the_key_after_the_machine(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let created = app.create_app(owner, workspace, true).await.unwrap();
    let url = format!("http://{}", app.addr);

    // when
    let config = scylla_agent::join(&url, &created.join_code).await.unwrap();

    // then
    let keys = app.get_app(owner, &config.app_id).await.unwrap().keys;
    assert!(!keys[0].label.is_empty());
}

/// A first start with a join code and no file joins and writes the file. A second start reads
/// the file and does not use the code again.
#[sqlx::test(migrations = false)]
async fn a_first_start_joins_one_time(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let created = app.create_app(owner, workspace, true).await.unwrap();
    let url = format!("http://{}", app.addr);
    let root = std::env::temp_dir().join(format!("scylla-agent-{}", ActorId::generate()));
    let path = root.join("agent.json");
    let join_with = Some((url.as_str(), created.join_code.as_str()));

    // when
    let (first, joined) = scylla_agent::load_or_join(&path, join_with).await.unwrap();
    let (second, again) = scylla_agent::load_or_join(&path, join_with).await.unwrap();

    // then
    let _ = std::fs::remove_dir_all(&root);
    assert!(joined && !again);
    assert_eq!((first.app_id, first.key_id), (second.app_id, second.key_id));
}
