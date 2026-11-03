//! The settings of a running agent.

use std::path::PathBuf;

/// How the agent runs its jobs.
#[derive(Clone, Debug)]
pub struct Settings {
    /// The folder that holds a work folder for each job.
    pub workspace_root: PathBuf,
    /// The largest number of jobs at the same time.
    pub capacity: u32,
    /// The container runtime, for example `docker`, or `None` for no container.
    pub container_runtime: Option<String>,
    /// `true` keeps the work folder of a job after its end.
    pub keep_workspace: bool,
}
