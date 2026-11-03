//! The loop of the streams: the agent connects again after each end of its stream, with a
//! growing wait, until a refusal for good or a stop.

use std::future::Future;
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use super::{AgentError, Settings};
use crate::config::AgentConfig;
use crate::jobs::JobSet;
use crate::stream::{session, Shared};

/// The first wait before a new connection.
const FIRST_WAIT: Duration = Duration::from_secs(1);

/// The longest wait before a new connection.
const LONGEST_WAIT: Duration = Duration::from_secs(60);

/// The time of a stream after which the wait starts again from the first wait.
const STABLE: Duration = Duration::from_secs(30);

/// The number of messages of the jobs that wait for a stream.
const INBOX: usize = 10_000;

/// Runs the agent until `stop` ends or the server refuses the agent for good. The jobs keep
/// running while the agent connects again. At the stop, the agent stops its jobs.
pub async fn run(
    config: AgentConfig,
    settings: Settings,
    stop: impl Future<Output = ()>,
) -> Result<(), AgentError> {
    // Share the jobs and their messages between the sessions.
    let (outbox, inbox) = mpsc::channel(INBOX);
    let mut shared = Shared {
        config,
        settings,
        jobs: JobSet::default(),
        outbox,
        inbox,
        pending: None,
    };
    let (stopping, stopped) = watch::channel(false);
    let stop = async move {
        stop.await;
        let _ = stopping.send(true);
    };
    tokio::pin!(stop);

    // Open a stream again after each end, with a growing wait.
    let mut wait = FIRST_WAIT;
    let result = loop {
        let start = Instant::now();
        let ended = tokio::select! {
            ended = session(&mut shared, stopped.clone()) => ended,
            () = &mut stop => Ok(()),
        };
        if *stopped.borrow() {
            break Ok(());
        }
        match ended {
            Err(AgentError::Refused(reason)) => break Err(AgentError::Refused(reason)),
            Err(error) => tracing::warn!(%error, "the stream ended"),
            Ok(()) => {}
        }
        if start.elapsed() > STABLE {
            wait = FIRST_WAIT;
        }
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            () = &mut stop => break Ok(()),
        }
        wait = (wait * 2).min(LONGEST_WAIT);
    };

    // Stop the jobs, and wait at most 10 seconds for their end.
    shared.jobs.cancel_all();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !shared.jobs.is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    result
}
