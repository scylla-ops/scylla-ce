//! The background tasks of an edition: the server runs each one at its period, from its start to
//! its stop.

use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sqlx::PgPool;

use crate::{Edition, EditionBuilder};

/// Why one run of a task failed. The server logs it, and runs the task again at the next period.
pub type TaskError = Box<dyn Error + Send + Sync>;

/// A background task of an edition, for example the purge of old rows or a count of the usage.
#[async_trait]
pub trait Task: Send + Sync + 'static {
    /// Runs the task one time. It begins and commits its own transactions.
    async fn run(&self, ctx: &TaskContext) -> Result<(), TaskError>;
}

/// What a task gets at each run.
#[non_exhaustive]
pub struct TaskContext {
    /// The database pool.
    pub pool: PgPool,
}

impl TaskContext {
    /// Makes the context of the runs.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// A task of the edition, with its name and its period.
#[derive(Clone)]
pub struct ScheduledTask {
    /// The name of the task in the log, `<subject>.<action>`: `feedback.purge`.
    pub name: &'static str,
    /// The time between two runs. The first run starts with the server.
    pub every: Duration,
    /// The task.
    pub task: Arc<dyn Task>,
}

impl Edition {
    /// Returns the background tasks that the server runs.
    pub fn tasks(&self) -> &[ScheduledTask] {
        &self.tasks
    }
}

impl EditionBuilder {
    /// Adds a background task: the server runs it at its start, then each `every`.
    pub fn task(mut self, name: &'static str, every: Duration, task: Arc<dyn Task>) -> Self {
        let scheduled = ScheduledTask { name, every, task };
        self.edition.tasks.push(scheduled);
        self
    }
}
