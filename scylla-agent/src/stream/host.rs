//! The machine of the agent, as its hello tells it.

use std::time::SystemTime;

use scylla_proto::apps::v1::Host;

use super::machine::Machine;
use crate::agent::Settings;
use crate::jobs::{features, Git};

/// Reads the facts of the machine, the features of the git that it has, and the start of the
/// agent.
pub(super) fn host(
    settings: &Settings,
    git: Option<Git>,
    machine: &mut Machine,
    started_at: SystemTime,
) -> Host {
    let hostname = hostname().unwrap_or_else(|| "unknown".to_owned());
    let cpu_count = std::thread::available_parallelism()
        .map(|count| count.get() as u32)
        .unwrap_or(1);
    Host {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        hostname,
        cpu_count,
        memory_bytes: machine.memory(),
        capacity: settings.capacity,
        container_runtime: settings.container_runtime.clone().unwrap_or_default(),
        features: features(git),
        started_at: Some(started_at.into()),
    }
}

/// Returns the name of the machine, or `None` if it has no name.
pub(crate) fn hostname() -> Option<String> {
    let name = gethostname::gethostname();
    let name = name.to_string_lossy().trim().to_owned();
    (!name.is_empty()).then_some(name)
}
