//! The tests of the background tasks of an edition: the server runs each one at its start with its
//! pool, and a failed run goes to the log.

use std::sync::Arc;
use std::time::Duration;

use scylla_core::infra::db::MIGRATOR;
use scylla_domain::identity::actors::Actor;
use scylla_extension::{async_trait, Edition, Task, TaskContext, TaskError};
use scylla_testkit::{Logs, TestApp};
use sqlx::PgPool;
use tokio::sync::mpsc;

/// The period of the tasks of the tests: no second run comes during a test.
const HOUR: Duration = Duration::from_secs(60 * 60);

/// The longest wait for a run.
const WAIT: Duration = Duration::from_secs(10);

/// A task that reads one value with the pool of its run, and sends it.
struct ReadOne(mpsc::UnboundedSender<i32>);

#[async_trait]
impl Task for ReadOne {
    async fn run(&self, ctx: &TaskContext) -> Result<(), TaskError> {
        let one = sqlx::query_scalar("SELECT 1").fetch_one(&ctx.pool).await?;
        self.0.send(one)?;
        Ok(())
    }
}

/// A task that fails at each run.
struct Fail;

#[async_trait]
impl Task for Fail {
    async fn run(&self, _ctx: &TaskContext) -> Result<(), TaskError> {
        Err("the database is far".into())
    }
}

/// Starts an edition with only the actors and one task.
async fn spawn(pool: PgPool, name: &'static str, task: Arc<dyn Task>) -> TestApp {
    let edition = Edition::builder("test")
        .migrations(&MIGRATOR)
        .authz::<Actor>()
        .task(name, HOUR, task)
        .build();
    TestApp::spawn(pool, edition).await
}

/// The server runs a task of the edition at its start, with the pool of the server.
#[sqlx::test(migrations = false)]
async fn a_task_runs_at_the_start_with_the_pool(pool: PgPool) {
    // given
    let (sender, mut runs) = mpsc::unbounded_channel();

    // when
    let _app = spawn(pool, "test.read", Arc::new(ReadOne(sender))).await;

    // then
    let run = tokio::time::timeout(WAIT, runs.recv()).await;
    assert_eq!(run.unwrap(), Some(1));
}

/// A failed run logs a warning with the name of the task and the error.
#[sqlx::test(migrations = false)]
async fn a_failed_run_logs_its_task_and_its_error(pool: PgPool) {
    // given
    let logs = Logs::json();

    // when
    let _app = spawn(pool, "test.fail", Arc::new(Fail)).await;

    // then
    let logged = tokio::time::timeout(WAIT, async {
        while !logs.text().contains("the database is far") {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    let text = logs.text();
    assert!(logged.is_ok(), "{text}");
    let line = text
        .lines()
        .find(|line| line.contains("the database is far"));
    let line = line.unwrap();
    assert!(line.contains("\"level\":\"warn\""), "{line}");
    assert!(line.contains("\"task\":\"test.fail\""), "{line}");
}
