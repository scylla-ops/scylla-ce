use scylla_proto::agent::v1::AgentHello;
use scylla_proto::common::v1 as common;

pub fn hello(running_jobs: Vec<String>) -> AgentHello {
    AgentHello {
        version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        hostname: hostname(),
        cpu_count: cpu_count(),
        total_memory_mb: total_memory_mb(),
        running_jobs: running_jobs
            .into_iter()
            .map(|value| common::JobId { value })
            .collect(),
    }
}

fn cpu_count() -> i32 {
    std::thread::available_parallelism()
        .ok()
        .and_then(|n| i32::try_from(n.get()).ok())
        .unwrap_or(0)
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn hostname() -> String {
    let mut buf = vec![0u8; 256];
    // SAFETY: 256 covers the POSIX 255-byte cap plus terminator, and we pass
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return String::new();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    buf.truncate(end);
    String::from_utf8_lossy(&buf).into_owned()
}

#[cfg(not(unix))]
fn hostname() -> String {
    String::new()
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn total_memory_mb() -> i64 {
    // SAFETY: sysconf(3) takes no pointers and only reads static system limits.
    let (pages, page_size) = unsafe {
        (
            libc::sysconf(libc::_SC_PHYS_PAGES),
            libc::sysconf(libc::_SC_PAGE_SIZE),
        )
    };
    if pages <= 0 || page_size <= 0 {
        return 0;
    }
    pages.saturating_mul(page_size).saturating_div(1024 * 1024)
}

#[cfg(not(unix))]
fn total_memory_mb() -> i64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_describes_the_build_and_the_running_jobs() {
        let h = hello(vec!["job-1".into()]);
        assert_eq!(h.version, env!("CARGO_PKG_VERSION"));
        assert!(!h.os.is_empty());
        assert!(!h.arch.is_empty());
        assert!(h.cpu_count >= 0);
        assert!(h.total_memory_mb >= 0);
        assert_eq!(
            h.running_jobs,
            [common::JobId {
                value: "job-1".into()
            }]
        );
    }

    #[cfg(unix)]
    #[test]
    fn probed_values_are_populated_on_unix() {
        let h = hello(Vec::new());
        assert!(h.cpu_count > 0, "available_parallelism returned nothing");
        assert!(h.total_memory_mb > 0, "sysconf returned no memory");
        assert!(!h.hostname.is_empty(), "gethostname returned nothing");
    }
}
