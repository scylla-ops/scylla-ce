use chrono::{DateTime, Utc};
use prost_types::Timestamp;

use scylla_domain::JobEvent;
use scylla_domain::domain::job::LogStream;
use scylla_domain::domain::pipeline::Shell;

use crate::agent::v1::JobStatus;
use crate::agent::v1::job_status::{
    Event, JobCompleted, JobFailed, JobStarted, NodeCompleted, NodeFailed, NodeSkipped, NodeStarted,
};
use crate::common::v1 as common;
use crate::exec::v1 as exec;

#[must_use]
pub fn timestamp(dt: DateTime<Utc>) -> Option<Timestamp> {
    Some(Timestamp {
        seconds: dt.timestamp(),
        nanos: i32::try_from(dt.timestamp_subsec_nanos()).unwrap_or(0),
    })
}

#[must_use]
pub fn job_event_to_status(job_id: &str, event: JobEvent, at: DateTime<Utc>) -> JobStatus {
    let node = |value: String| Some(common::NodeId { value });
    let event = match event {
        JobEvent::JobStarted => Event::JobStarted(JobStarted {}),
        JobEvent::NodeStarted { node_id } => Event::NodeStarted(NodeStarted {
            node_id: node(node_id),
        }),
        JobEvent::NodeCompleted { node_id } => Event::NodeCompleted(NodeCompleted {
            node_id: node(node_id),
        }),
        JobEvent::NodeFailed { node_id, error } => Event::NodeFailed(NodeFailed {
            node_id: node(node_id),
            error,
        }),
        JobEvent::NodeSkipped { node_id } => Event::NodeSkipped(NodeSkipped {
            node_id: node(node_id),
        }),
        JobEvent::JobCompleted => Event::JobCompleted(JobCompleted {}),
        JobEvent::JobFailed { error } => Event::JobFailed(JobFailed { error }),
    };
    JobStatus {
        job_id: Some(common::JobId {
            value: job_id.to_string(),
        }),
        timestamp: timestamp(at),
        event: Some(event),
    }
}

#[must_use]
pub fn status_to_job_event(status: &JobStatus) -> Option<JobEvent> {
    let node_id = |id: &Option<common::NodeId>| id.clone().unwrap_or_default().value;
    Some(match status.event.as_ref()? {
        Event::JobStarted(_) => JobEvent::JobStarted,
        Event::NodeStarted(e) => JobEvent::NodeStarted {
            node_id: node_id(&e.node_id),
        },
        Event::NodeCompleted(e) => JobEvent::NodeCompleted {
            node_id: node_id(&e.node_id),
        },
        Event::NodeFailed(e) => JobEvent::NodeFailed {
            node_id: node_id(&e.node_id),
            error: e.error.clone(),
        },
        Event::NodeSkipped(e) => JobEvent::NodeSkipped {
            node_id: node_id(&e.node_id),
        },
        Event::JobCompleted(_) => JobEvent::JobCompleted,
        Event::JobFailed(e) => JobEvent::JobFailed {
            error: e.error.clone(),
        },
    })
}

#[must_use]
pub const fn log_stream_to_proto(stream: LogStream) -> common::LogStream {
    match stream {
        LogStream::Stdout => common::LogStream::Stdout,
        LogStream::Stderr => common::LogStream::Stderr,
    }
}

#[must_use]
pub fn log_stream_from_proto(raw: i32) -> LogStream {
    match common::LogStream::try_from(raw) {
        Ok(common::LogStream::Stderr) => LogStream::Stderr,
        _ => LogStream::Stdout,
    }
}

#[must_use]
pub const fn shell_to_proto(shell: Shell) -> exec::Shell {
    match shell {
        Shell::Sh => exec::Shell::Sh,
        Shell::Bash => exec::Shell::Bash,
    }
}

#[must_use]
pub fn shell_from_proto(raw: i32) -> Shell {
    match exec::Shell::try_from(raw) {
        Ok(exec::Shell::Bash) => Shell::Bash,
        _ => Shell::Sh,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_variant() -> Vec<JobEvent> {
        let node = || "build".to_string();
        vec![
            JobEvent::JobStarted,
            JobEvent::NodeStarted { node_id: node() },
            JobEvent::NodeCompleted { node_id: node() },
            JobEvent::NodeFailed {
                node_id: node(),
                error: "exit 1".to_string(),
            },
            JobEvent::NodeSkipped { node_id: node() },
            JobEvent::JobCompleted,
            JobEvent::JobFailed {
                error: "cancelled".to_string(),
            },
        ]
    }

    #[test]
    fn every_job_event_survives_a_round_trip() {
        for event in every_variant() {
            let wire = job_event_to_status("job-1", event.clone(), Utc::now());
            let back = status_to_job_event(&wire).expect("a built status always carries its event");
            assert_eq!(
                format!("{event:?}"),
                format!("{back:?}"),
                "{event:?} did not survive the round trip"
            );
        }
    }

    #[test]
    fn every_variant_is_covered() {
        assert_eq!(
            every_variant().len(),
            7,
            "add the new JobEvent variant here"
        );
    }

    #[test]
    fn a_status_without_an_event_is_rejected() {
        let empty = JobStatus {
            job_id: None,
            timestamp: None,
            event: None,
        };
        assert!(status_to_job_event(&empty).is_none());
    }

    #[test]
    fn a_status_carries_the_time_of_its_event() {
        let at = DateTime::from_timestamp(1_700_000_000, 123_456_000).unwrap();
        let wire = job_event_to_status("job-1", JobEvent::JobStarted, at);
        assert_eq!(wire.timestamp, timestamp(at));
    }

    #[test]
    fn log_streams_round_trip() {
        for stream in [LogStream::Stdout, LogStream::Stderr] {
            let raw = log_stream_to_proto(stream) as i32;
            assert_eq!(log_stream_from_proto(raw), stream);
        }
        assert_eq!(log_stream_from_proto(0), LogStream::Stdout);
        assert_eq!(log_stream_from_proto(99), LogStream::Stdout);
    }

    #[test]
    fn shells_round_trip() {
        for shell in [Shell::Sh, Shell::Bash] {
            let raw = shell_to_proto(shell) as i32;
            assert_eq!(shell_from_proto(raw), shell);
        }
        assert_eq!(shell_from_proto(0), Shell::Sh);
        assert_eq!(shell_from_proto(99), Shell::Sh);
    }
}
