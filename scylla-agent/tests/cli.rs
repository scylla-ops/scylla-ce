//! The tests of the command line of `scylla-agent` against the real server of the Community
//! edition: its errors, its exit codes, its file, and its container runtime.

use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::Duration;

use scylla_agent::AgentConfig;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::pipelines::PipelineHolder;
use scylla_testkit::TestApp;
use sqlx::PgPool;
use tokio::process::Command;

/// The exit code of a refusal for good.
const REFUSED: i32 = 78;

/// A folder of a test. The end of the test deletes it.
struct Folder(PathBuf);

impl Folder {
    /// Makes an empty folder.
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("scylla-agent-{}", ActorId::generate()));
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
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The command of the binary, with no setting of the agent from the environment of the test.
fn binary() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_scylla-agent"));
    for name in [
        "SCYLLA_URL",
        "SCYLLA_JOIN_CODE",
        "SCYLLA_AGENT_CONFIG",
        "SCYLLA_AGENT_WORKSPACE_ROOT",
        "SCYLLA_AGENT_CONTAINER_RUNTIME",
    ] {
        command.env_remove(name);
    }
    command.kill_on_drop(true);
    command
}

/// Runs `join` with a file.
async fn join(file: &Path, url: &str, code: &str, more: &[&str]) -> Output {
    let mut command = binary();
    command.arg("--config").arg(file);
    command.args(["join", url, code]).args(more);
    command.output().await.unwrap()
}

/// The standard error of a process.
fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Starts the server, and returns it with a new user and its URL.
async fn server(pool: PgPool) -> (TestApp, ActorId, String) {
    let app = TestApp::spawn(pool, scylla_ce::edition()).await;
    let owner = app.user().await;
    let url = format!("http://{}", app.addr);
    (app, owner, url)
}

/// Creates an App that runs jobs, and returns its join code.
async fn new_code(app: &TestApp, owner: ActorId) -> String {
    let workspace = app.workspace_of(owner).await;
    let created = app.create_app(owner, workspace, true).await.unwrap();
    created.join_code
}

/// A used code tells the reason in words, with the exit code of a refusal.
#[sqlx::test(migrations = false)]
async fn a_used_code_tells_the_reason_and_exits_78(pool: PgPool) {
    // given
    let (app, owner, url) = server(pool).await;
    let code = new_code(&app, owner).await;
    let folder = Folder::new();
    let first = join(&folder.file("first.json"), &url, &code, &[]).await;
    assert!(first.status.success(), "{}", stderr(&first));

    // when
    let used = join(&folder.file("second.json"), &url, &code, &[]).await;

    // then
    let text = stderr(&used);
    assert_eq!(used.status.code(), Some(REFUSED), "{text}");
    assert!(
        text.contains("the join code is wrong, used or expired"),
        "{text}"
    );
    assert!(!text.contains("Refused("), "{text}");
}

/// A file that cannot be written stops `join` and `run` before they use the code.
#[sqlx::test(migrations = false)]
async fn a_file_that_cannot_be_written_keeps_the_code(pool: PgPool) {
    // given
    let (app, owner, url) = server(pool).await;
    let code = new_code(&app, owner).await;
    let folder = Folder::new();
    let blocker = folder.file("blocker");
    std::fs::write(&blocker, "").unwrap();
    let unwritable = blocker.join("agent.json");

    // when
    let joined = join(&unwritable, &url, &code, &[]).await;
    let mut start = binary();
    start.arg("--config").arg(&unwritable).arg("run");
    start.env("SCYLLA_URL", &url).env("SCYLLA_JOIN_CODE", &code);
    let started = start.output().await.unwrap();
    let retried = join(&folder.file("agent.json"), &url, &code, &[]).await;

    // then
    let text = stderr(&joined);
    assert_eq!(joined.status.code(), Some(1), "{text}");
    assert!(text.contains(&unwritable.display().to_string()), "{text}");
    assert_eq!(started.status.code(), Some(1), "{}", stderr(&started));
    assert!(retried.status.success(), "{}", stderr(&retried));
}

/// `join` keeps a file that exists and its code, and `--force` replaces the file with a new key.
#[sqlx::test(migrations = false)]
async fn a_join_replaces_a_file_only_with_force(pool: PgPool) {
    // given
    let (app, owner, url) = server(pool).await;
    let code = new_code(&app, owner).await;
    let folder = Folder::new();
    let file = folder.file("agent.json");
    let first = join(&file, &url, &code, &[]).await;
    assert!(first.status.success(), "{}", stderr(&first));
    let before = AgentConfig::load(&file).unwrap();
    let code = app.create_join_code(owner, &before.app_id).await.unwrap();

    // when
    let refused = join(&file, &url, &code, &[]).await;
    let kept = AgentConfig::load(&file).unwrap();
    let forced = join(&file, &url, &code, &["--force"]).await;
    let replaced = AgentConfig::load(&file).unwrap();

    // then
    let text = stderr(&refused);
    assert_eq!(refused.status.code(), Some(1), "{text}");
    assert!(text.contains("--force"), "{text}");
    assert_eq!(kept.key_id, before.key_id);
    assert!(forced.status.success(), "{}", stderr(&forced));
    assert_ne!(replaced.key_id, before.key_id);
}

/// A closed port names the address and the cause.
#[tokio::test]
async fn a_closed_port_names_the_address_and_the_cause() {
    // given
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let folder = Folder::new();

    // when
    let failed = join(&folder.file("agent.json"), &url, "code", &[]).await;

    // then
    let text = stderr(&failed);
    assert_eq!(failed.status.code(), Some(1), "{text}");
    assert!(text.contains(&url), "{text}");
    assert!(text.to_lowercase().contains("refused"), "{text}");
}

/// A start with the URL alone and no file tells what to do, and is not an error of the options.
#[tokio::test]
async fn the_url_alone_and_no_file_tell_what_to_do() {
    // given
    let folder = Folder::new();
    let file = folder.file("agent.json");
    let mut start = binary();
    start.arg("--config").arg(&file).arg("run");
    start.env("SCYLLA_URL", "http://127.0.0.1:1");

    // when
    let failed = start.output().await.unwrap();

    // then
    let text = stderr(&failed);
    assert_eq!(failed.status.code(), Some(1), "{text}");
    assert!(text.contains("does not exist"), "{text}");
    assert!(text.contains("scylla-agent join"), "{text}");
}

/// A restart with the URL alone reads the file, and a revoked key stops it with the exit code of
/// a refusal.
#[sqlx::test(migrations = false)]
async fn a_revoked_key_stops_a_restart_with_78(pool: PgPool) {
    // given
    let (app, owner, url) = server(pool).await;
    let code = new_code(&app, owner).await;
    let folder = Folder::new();
    let file = folder.file("agent.json");
    let joined = join(&file, &url, &code, &[]).await;
    assert!(joined.status.success(), "{}", stderr(&joined));
    let config = AgentConfig::load(&file).unwrap();
    app.revoke_app_key(owner, &config.app_id, &config.key_id)
        .await
        .unwrap();
    let mut start = binary();
    start
        .arg("--config")
        .arg(&file)
        .args(["run", "--workspace-root"]);
    start.arg(folder.file("work")).env("SCYLLA_URL", &url);

    // when
    let stopped = tokio::time::timeout(Duration::from_secs(30), start.output())
        .await
        .expect("the agent did not stop")
        .unwrap();

    // then
    let text = stderr(&stopped);
    assert_eq!(stopped.status.code(), Some(REFUSED), "{text}");
    assert!(text.contains("the server refuses the agent"), "{text}");
}

/// Starts the agent of a new App with a `PATH` of one folder that has `programs`, and returns the
/// container runtime that its hello tells.
async fn runtime_of(
    app: &TestApp,
    owner: ActorId,
    url: &str,
    programs: &[&str],
    more: &[&str],
) -> String {
    let workspace = app.workspace_of(owner).await;
    let created = app.create_app(owner, workspace, true).await.unwrap();
    let folder = Folder::new();
    let file = folder.file("agent.json");
    let joined = join(&file, url, &created.join_code, &[]).await;
    assert!(joined.status.success(), "{}", stderr(&joined));
    let config = AgentConfig::load(&file).unwrap();
    let id = ActorId::parse(&config.app_id).unwrap();
    app.grant(&[PipelineHolder::RUNNER.of(workspace, id)]).await;
    for program in programs {
        std::fs::write(folder.file(program), "").unwrap();
    }
    let mut start = binary();
    start.env("PATH", &folder.0).arg("--config").arg(&file);
    start.args(["run", "--workspace-root"]);
    start.arg(folder.file("work")).args(more);
    let _running = start.stderr(Stdio::null()).spawn().unwrap();
    for _ in 0..300 {
        let read = app.get_app(owner, &config.app_id).await.unwrap();
        if let Some(host) = read
            .app
            .filter(|app| app.connected)
            .and_then(|app| app.host)
        {
            return host.container_runtime;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the agent did not connect");
}

/// `auto` is the default and finds docker, then podman, in the `PATH`, and `none` turns it off.
#[sqlx::test(migrations = false)]
async fn auto_finds_the_container_runtime_and_none_turns_it_off(pool: PgPool) {
    // given
    let (app, owner, url) = server(pool).await;

    // when
    let both = runtime_of(&app, owner, &url, &["docker", "podman"], &[]).await;
    let podman = runtime_of(&app, owner, &url, &["podman"], &[]).await;
    let missing = runtime_of(&app, owner, &url, &[], &[]).await;
    let none = ["--container-runtime", "none"];
    let off = runtime_of(&app, owner, &url, &["docker"], &none).await;

    // then
    assert_eq!([both, podman, missing, off], ["docker", "podman", "", ""]);
}
