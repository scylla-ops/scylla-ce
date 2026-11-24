//! The machine of the agent: its memory for the hello, and the samples of its use that the agent
//! sends at the interval of the welcome. The crate `sysinfo` reads them on each system.

use std::time::Duration;

use scylla_proto::agents::v1::Metrics;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
use tokio::time::{interval, Interval, MissedTickBehavior};

/// The machine, with its last read of the CPUs: the CPU use of a sample is the use since the
/// read before.
pub(crate) struct Machine {
    /// The last reads of the system.
    system: System,
}

impl Machine {
    /// Reads the CPUs and the memory a first time.
    pub(crate) fn new() -> Self {
        let refresh = RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
            .with_memory(MemoryRefreshKind::nothing().with_ram());
        Self {
            system: System::new_with_specifics(refresh),
        }
    }

    /// Returns the memory of the machine in bytes.
    pub(crate) fn memory(&mut self) -> u64 {
        self.system.refresh_memory();
        self.system.total_memory()
    }

    /// Reads a sample: the CPU use of all the CPUs since the read before, and the used memory.
    pub(crate) fn sample(&mut self) -> Metrics {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        Metrics {
            cpu_percent: self.system.global_cpu_usage().clamp(0.0, 100.0),
            memory_used_bytes: self.system.used_memory(),
        }
    }
}

/// Returns the timer of the samples for the interval of a welcome in seconds, or `None` for 0:
/// the server then takes no sample. The first tick comes at once.
pub(super) fn timer(seconds: u32) -> Option<Interval> {
    (seconds > 0).then(|| {
        let mut timer = interval(Duration::from_secs(u64::from(seconds)));
        timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
        timer
    })
}

/// Waits for the next tick of a timer, or for ever with no timer.
pub(super) async fn tick(timer: &mut Option<Interval>) {
    match timer {
        Some(timer) => {
            timer.tick().await;
        }
        None => std::future::pending().await,
    }
}
