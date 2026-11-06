//! The tests of a container step against the real server of the Community edition, with a small
//! script as the program of the container runtime: the settings that the program gets, and the
//! delete of a work folder after a container wrote a file in it.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use scylla_agent::AgentConfig;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::pipelines::PipelineHolder;
use scylla_proto::pipelines::v1::{node, Container, Exec, Job, JobStatus};
use scylla_testkit::{exec_node, text, TestApp};
use sqlx::PgPool;
use tokio::process::{Child, Command};

/// A folder of a test. The end of the test deletes it.
struct Folder(PathBuf);

impl Folder {
    /// Makes an empty folder.
    fn new() -> Self {
        let name = format!("scylla-agent-{}", ActorId::generate());
        let path = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /// The path of a name in the folder.
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::process::Command::new("chmod")
            .arg("-R")
            .arg("u+w")
            .arg(&self.0)
            .status();
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Joins a new App, and starts the agent with a script as its container runtime.
async fn start_agent(
    app: &TestApp,
    owner: ActorId,
    folder: &Folder,
    runtime: &str,
    docker_host: &str,
) -> Child {
    let program = folder.file("fake-runtime");
    let script = runtime.replace("@FOLDER@", &folder.0.display().to_string());
    std::fs::write(&program, script).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let workspace = app.workspace_of(owner).await;
    let created = app.create_app(owner, workspace, true).await.unwrap();
    let file = folder.file("agent.json");
    let url = format!("http://{}", app.addr);
    scylla_agent::join_and_save(&file, &url, &created.join_code)
        .await
        .unwrap();
    let config = AgentConfig::load(&file).unwrap();
    let id = ActorId::parse(&config.app_id).unwrap();
    app.grant(&[PipelineHolder::RUNNER.of(workspace, id)]).await;
    let path = std::env::var("PATH").unwrap_or_default();
    Command::new(env!("CARGO_BIN_EXE_scylla-agent"))
        .env("DOCKER_HOST", docker_host)
        .env("PATH", format!("{}:{path}", folder.0.display()))
        .env_remove("SCYLLA_URL")
        .env_remove("SCYLLA_JOIN_CODE")
        .arg("--config")
        .arg(&file)
        .args(["run", "--workspace-root"])
        .arg(folder.file("work"))
        .args(["--container-runtime", "fake-runtime"])
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

/// Runs one container step, and waits for the end of its job.
async fn run_container_step(app: &TestApp, owner: ActorId) -> Job {
    let project = text(&app.create_project(owner, "alpha").await.unwrap().id).to_owned();
    let mut step = exec_node("step", &[]);
    step.step = Some(node::Step::Exec(Exec {
        argv: vec!["true".to_owned()],
    }));
    step.container = Some(Container {
        image: "alpine".to_owned(),
    });
    let pipeline = app
        .create_pipeline(owner, &project, vec![step])
        .await
        .unwrap();
    let job = app.run_pipeline(owner, text(&pipeline.id)).await.unwrap();
    let id = text(&job.id).to_owned();
    for _ in 0..300 {
        let read = app.get_job(owner, &id).await.unwrap();
        if read.status() == JobStatus::Completed || read.status() == JobStatus::Failed {
            return read;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the job did not end");
}

/// The program of the runtime reaches its daemon with the settings of the agent, and the container
/// gets only the names of the job, so these values stay out of the job.
#[sqlx::test(migrations = false)]
async fn the_runtime_gets_its_settings_and_the_container_does_not(pool: PgPool) {
    // given
    let app = TestApp::spawn(pool, scylla_ce::edition()).await;
    let owner = app.user().await;
    let folder = Folder::new();
    let runtime = "#!/bin/sh\nenv > @FOLDER@/env\necho \"$@\" > @FOLDER@/args\n";
    let _agent = start_agent(&app, owner, &folder, runtime, "tcp://daemon:2375").await;

    // when
    let job = run_container_step(&app, owner).await;

    // then
    assert_eq!(job.status(), JobStatus::Completed);
    let env = std::fs::read_to_string(folder.file("env")).unwrap_or_default();
    let args = std::fs::read_to_string(folder.file("args")).unwrap_or_default();
    assert!(env.contains("DOCKER_HOST=tcp://daemon:2375"), "{env}");
    assert!(args.contains("-e CI"), "{args}");
    assert!(!args.contains("DOCKER_HOST"), "{args}");
    assert!(!args.contains("daemon"), "{args}");
}

/// A file that the agent cannot delete stops the delete of the work folder, so a container of the
/// same image empties the folder, and the folder goes.
#[sqlx::test(migrations = false)]
async fn a_container_empties_a_work_folder_that_the_agent_cannot_delete(pool: PgPool) {
    // given
    let app = TestApp::spawn(pool, scylla_ce::edition()).await;
    let owner = app.user().await;
    let folder = Folder::new();
    let runtime = r#"#!/bin/sh
for argument in "$@"; do
  case "$argument" in
    *:/work) dir=${argument%:/work} ;;
    --entrypoint) cleaning=1 ;;
  esac
done
if [ -n "${cleaning:-}" ]; then
  echo "$@" > @FOLDER@/cleaned
  chmod -R u+w "$dir"
  rm -rf "$dir"/*
else
  mkdir "$dir/locked"
  touch "$dir/locked/file"
  chmod 555 "$dir/locked"
fi
"#;
    let _agent = start_agent(&app, owner, &folder, runtime, "unix:///none").await;

    // when
    let job = run_container_step(&app, owner).await;

    // then
    assert_eq!(job.status(), JobStatus::Completed);
    let mut left = 1;
    for _ in 0..100 {
        left = std::fs::read_dir(folder.file("work"))
            .map(|entries| entries.count())
            .unwrap_or(0);
        if left == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(left, 0, "the work folder stays");
    let cleaned = std::fs::read_to_string(folder.file("cleaned")).unwrap();
    assert!(cleaned.contains("--entrypoint find alpine"), "{cleaned}");
    assert!(cleaned.contains("/work -mindepth 1 -delete"), "{cleaned}");
}
