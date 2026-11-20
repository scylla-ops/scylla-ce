//! The tests of the machine of the agent against the real server of the Community edition: the
//! start and the memory in the hello, and the first sample after the welcome.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use scylla_agent::Settings;
use scylla_domain::identity::actors::ActorId;
use scylla_testkit::TestApp;
use sqlx::PgPool;
use tokio::sync::oneshot;

/// The hello tells the start of the agent and the memory of its machine, and the agent sends a
/// sample at once after the welcome.
#[sqlx::test(migrations = false)]
async fn the_agent_tells_its_machine(pool: PgPool) {
    // given
    let app = TestApp::spawn(pool, scylla_ce::edition()).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let created = app.create_app(owner, workspace, true).await.unwrap();
    let url = format!("http://{}", app.addr);
    let config = scylla_agent::join(&url, &created.join_code).await.unwrap();
    let id = config.app_id.clone();
    let root = std::env::temp_dir().join(format!("scylla-agent-{}", ActorId::generate()));
    let settings = Settings {
        workspace_root: root.clone(),
        capacity: 1,
        container_runtime: None,
        keep_workspace: false,
    };
    let before = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let (stop, stopped) = oneshot::channel::<()>();

    // when
    let task = tokio::spawn(scylla_agent::run(config, settings, async {
        let _ = stopped.await;
    }));
    let mut read = app.get_app(owner, &id).await.unwrap().app.unwrap();
    for _ in 0..100 {
        if read.last_sample.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        read = app.get_app(owner, &id).await.unwrap().app.unwrap();
    }
    let _ = stop.send(());
    let _ = task.await;
    let _ = std::fs::remove_dir_all(&root);

    // then
    let host = read.host.expect("a host");
    let started = host.started_at.expect("a start");
    assert!(started.seconds >= i64::try_from(before.as_secs()).unwrap());
    assert!(host.memory_bytes > 0);
    let sample = read.last_sample.expect("a sample");
    assert!((0.0..=100.0).contains(&sample.cpu_percent));
    assert!(sample.memory_used_bytes > 0);
}
