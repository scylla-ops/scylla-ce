//! One session of the stream: a token, the hello, the messages in both directions, and the
//! renewal of the token before its expiry.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use scylla_proto::agents::v1::agent_service_client::AgentServiceClient;
use scylla_proto::agents::v1::open_stream_request::Message as Up;
use scylla_proto::agents::v1::open_stream_response::Message as Down;
use scylla_proto::agents::v1::{Hello, OpenStreamRequest, Renew};
use scylla_proto::common::v1::Id;
use tokio::sync::{mpsc, watch};
use tokio::time::{sleep_until, Instant};
use tokio_stream::wrappers::ReceiverStream;
use tonic::metadata::MetadataValue;
use tonic::{Code, Request};

use super::host::host;
use crate::agent::{AgentError, Settings};
use crate::config::AgentConfig;
use crate::identity::{connect, token};
use crate::jobs::{Git, JobSet, Outbox};

/// The time before the expiry of a token when the agent renews it.
const RENEW_BEFORE: i64 = 60;

/// What the sessions of the agent share.
pub(crate) struct Shared {
    /// The file of the agent.
    pub(crate) config: AgentConfig,
    /// The settings.
    pub(crate) settings: Settings,
    /// The running jobs.
    pub(crate) jobs: JobSet,
    /// The sender of the messages of the jobs.
    pub(crate) outbox: Outbox,
    /// The messages of the jobs, that wait for a stream.
    pub(crate) inbox: mpsc::Receiver<OpenStreamRequest>,
    /// A message that a closed stream did not take.
    pub(crate) pending: Option<OpenStreamRequest>,
}

/// Runs one session until the stream ends or the agent stops. A refusal for good gives an
/// error `Refused`.
pub(crate) async fn session(
    shared: &mut Shared,
    mut stop: watch::Receiver<bool>,
) -> Result<(), AgentError> {
    // Get a token, find the git of the machine, and open the stream with the hello.
    let channel = connect(&shared.config.url).await?;
    let mut current = token(channel.clone(), &shared.config).await?;
    let (up, messages) = mpsc::channel(64);
    let running = shared
        .jobs
        .running()
        .into_iter()
        .map(|value| Id { value })
        .collect();
    let git = Git::detect().await;
    let hello = Hello {
        host: Some(host(&shared.settings, git)),
        running_jobs: running,
    };
    let _ = up.send(message(Up::Hello(hello))).await;
    let mut request = Request::new(ReceiverStream::new(messages));
    let bearer = MetadataValue::try_from(format!("Bearer {}", current.value))
        .map_err(|error| AgentError::Connection(error.to_string()))?;
    request.metadata_mut().insert("authorization", bearer);
    let mut down = AgentServiceClient::new(channel.clone())
        .open_stream(request)
        .await
        .map_err(|status| AgentError::of_status(&status))?
        .into_inner();
    tracing::info!(app = %shared.config.app_id, "the stream is open");

    // Answer the server, send the messages of the jobs, and renew the token.
    loop {
        let renew = renew_at(current.expires_at);
        tokio::select! {
            received = down.message() => match received {
                Ok(Some(response)) => answer(shared, git, response.message),
                Ok(None) => return Ok(()),
                Err(status) if matches!(status.code(), Code::Unauthenticated | Code::Aborted) => {
                    tracing::info!(reason = status.message(), "the server closed the stream");
                    return Ok(());
                }
                Err(status) => return Err(AgentError::of_status(&status)),
            },
            next = next(shared) => {
                let Some(next) = next else { return Ok(()) };
                if let Err(error) = up.send(next).await {
                    shared.pending = Some(error.0);
                    return Ok(());
                }
            }
            () = sleep_until(renew) => {
                current = token(channel.clone(), &shared.config).await?;
                let renew = Renew { token: current.value.clone() };
                let _ = up.send(message(Up::Renew(renew))).await;
            }
            () = async { let _ = stop.wait_for(|stop| *stop).await; } => return Ok(()),
        }
    }
}

/// Acts on a message of the server: a dispatch starts a job with the git of the hello, and a
/// cancel stops one.
fn answer(shared: &Shared, git: Option<Git>, message: Option<Down>) {
    match message {
        Some(Down::Dispatch(dispatch)) => {
            let job = dispatch
                .job_id
                .as_ref()
                .map(|id| id.value.clone())
                .unwrap_or_default();
            tracing::info!(%job, "a job starts");
            let (settings, outbox) = (shared.settings.clone(), shared.outbox.clone());
            shared.jobs.start(dispatch, settings, git, outbox);
        }
        Some(Down::Cancel(cancel)) => {
            let job = cancel.job_id.map(|id| id.value).unwrap_or_default();
            tracing::info!(%job, "a job stops");
            shared.jobs.cancel(&job);
        }
        Some(Down::Welcome(_)) | Some(Down::Renewed(_)) | None => {}
    }
}

/// Returns the next message of the jobs: the one that a closed stream did not take first.
async fn next(shared: &mut Shared) -> Option<OpenStreamRequest> {
    match shared.pending.take() {
        Some(message) => Some(message),
        None => shared.inbox.recv().await,
    }
}

/// Makes a message of the agent.
fn message(message: Up) -> OpenStreamRequest {
    OpenStreamRequest {
        message: Some(message),
    }
}

/// Returns the instant of the renewal of a token: one minute before its expiry, and at least 5
/// seconds from now.
fn renew_at(expires_at: i64) -> Instant {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_secs() as i64)
        .unwrap_or_default();
    let left = (expires_at - RENEW_BEFORE - now).max(5);
    Instant::now() + Duration::from_secs(left as u64)
}
