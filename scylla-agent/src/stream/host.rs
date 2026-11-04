//! The machine of the agent, as its hello tells it.

use scylla_proto::apps::v1::Host;

use crate::agent::Settings;

/// Reads the facts of the machine.
pub(super) fn host(settings: &Settings) -> Host {
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
        memory_bytes: memory(),
        capacity: settings.capacity,
        container_runtime: settings.container_runtime.clone().unwrap_or_default(),
    }
}

/// Returns the name of the machine, or `None` if it has no name.
pub(crate) fn hostname() -> Option<String> {
    let name = gethostname::gethostname();
    let name = name.to_string_lossy().trim().to_owned();
    (!name.is_empty()).then_some(name)
}

/// Reads the memory of the machine from `/proc/meminfo`, or 0.
fn memory() -> u64 {
    let text = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    text.lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|kilobytes| kilobytes.parse::<u64>().ok())
        .map_or(0, |kilobytes| kilobytes * 1024)
}
