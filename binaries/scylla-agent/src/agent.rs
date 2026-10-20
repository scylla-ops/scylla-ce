use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, mpsc};
use tokio::task::{self, JoinSet};
use tokio_util::sync::CancellationToken;
use tonic::Streaming;
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use tonic::{Code, Request};
use tracing::{debug, error, info, warn};

use scylla_domain::JobEvent;
use scylla_domain::domain::pipeline::PipelineNode;
use scylla_domain::domain::pipeline::{EnvKey, EnvVar, NodeId, Step, WorkingDir};
use scylla_proto::agent::MAX_MESSAGE_BYTES;
use scylla_proto::agent::v1::agent_service_client::AgentServiceClient;
use scylla_proto::agent::v1::{
    AgentDown, AgentNode, AgentUp, JobDispatch, agent_down, agent_node, agent_up,
};
use scylla_proto::app::v1::IssueTokenRequest;
use scylla_proto::app::v1::app_auth_service_client::AppAuthServiceClient;
use scylla_proto::common::v1 as common;

use crate::config::AgentConfig;
use crate::error::{AgentError, ExecutionError};
use crate::executor::Executor;
use crate::output::Secrets;
use crate::reporter::StatusPublisher;

const MIN_UPTIME_FOR_RESET_SECS: u64 = 5;
const MAX_BACKOFF_SECS: u64 = 60;
/// The executor stops a job in `TERM_GRACE` (5 s); with the flush, the agent exits within 10 s.
const STOP_GRACE: Duration = Duration::from_secs(7);
const FLUSH: Duration = Duration::from_secs(3);

type Up = Arc<Mutex<mpsc::Receiver<AgentUp>>>;

pub struct Agent {
    config: AgentConfig,
    endpoint: Endpoint,
}

enum Served {
    Shutdown,
    Closed,
}

impl Agent {
    /// The control-plane URL is checked here, once: a URL that does not parse is a
    /// configuration error, not a connection to retry.
    pub fn new(config: AgentConfig) -> Result<Self, AgentError> {
        let url = config.control_plane_url.clone();
        // Keepalive: a half-open connection (NAT drop, control plane gone without FIN) must not hang the agent.
        let mut endpoint = Channel::from_shared(url.clone())
            .map_err(|e| AgentError::InvalidUrl {
                url: url.clone(),
                message: e.to_string(),
            })?
            .http2_keep_alive_interval(Duration::from_secs(20))
            .keep_alive_timeout(Duration::from_secs(10))
            .keep_alive_while_idle(true)
            .tcp_keepalive(Some(Duration::from_secs(30)));
        // https:// terminates at the proxy; load native roots. http:// stays h2c.
        if url.starts_with("https://") {
            endpoint = endpoint.tls_config(ClientTlsConfig::new().with_native_roots())?;
        }
        Ok(Self { config, endpoint })
    }

    /// Runs until `shutdown` fires or the control plane refuses the agent. A job keeps running
    /// across reconnects: its frames wait in one channel for the next stream. On the way out
    /// every job stops, and the last frames go out before the stream closes.
    /// The failure counter resets only after a connection that stayed up, so a
    /// connect-then-close loop still backs off.
    pub async fn run(&self, shutdown: CancellationToken) -> Result<(), AgentError> {
        let buffer = usize::try_from(self.config.publish_buffer_size).unwrap_or(8192);
        let (up_tx, up_rx) = mpsc::channel::<AgentUp>(buffer);
        let up_rx = Arc::new(Mutex::new(up_rx));
        let mut jobs = Jobs::new(shutdown.child_token(), up_tx);
        let base = self.config.reconnect_backoff_secs;
        let max = self.config.max_reconnect_attempts;
        let mut failures: u32 = 0;
        let mut open = None;

        let result = loop {
            let connected = tokio::select! {
                biased;
                () = shutdown.cancelled() => break Ok(()),
                connected = self.connect(&up_rx) => connected,
            };
            let uptime = match connected {
                Ok((mut inbound, retired)) => {
                    info!("agent stream open; waiting for jobs");
                    let started = Instant::now();
                    let served = self.serve(&mut inbound, &mut jobs, &shutdown).await;
                    if matches!(served, Ok(Served::Shutdown)) {
                        open = Some(inbound);
                        break Ok(());
                    }
                    retired.cancel();
                    served.map(|_| started.elapsed())
                }
                Err(e) => Err(e),
            };
            match uptime {
                Err(e) if is_terminal(&e) => {
                    error!(error = %e, "{}", refusal(&e));
                    break Err(e);
                }
                Ok(uptime) if uptime >= Duration::from_secs(MIN_UPTIME_FOR_RESET_SECS) => {
                    failures = 0;
                    info!("agent stream closed; reconnecting");
                }
                failed => {
                    failures += 1;
                    let e = failed.err().unwrap_or(AgentError::StreamClosed);
                    warn!(error = %e, attempt = failures, max, "no stable connection to the control plane; backing off");
                    if max != 0 && failures >= max {
                        error!("giving up after {failures} attempts");
                        break Err(e);
                    }
                }
            }
            tokio::select! {
                biased;
                () = shutdown.cancelled() => break Ok(()),
                () = tokio::time::sleep(backoff_delay(base, failures)) => {}
            }
        };

        jobs.stop().await;
        drop(jobs);
        if let Some(mut inbound) = open {
            let flushed = async { while let Ok(Some(_)) = inbound.message().await {} };
            if tokio::time::timeout(FLUSH, flushed).await.is_err() {
                warn!("the control plane did not close the stream in time");
            }
        }
        result
    }

    /// The frames go out from the one channel of the process. `retired` ends the stream of
    /// frames of this connection, so the next one takes the frames that this one did not send.
    async fn connect(
        &self,
        up_rx: &Up,
    ) -> Result<(Streaming<AgentDown>, CancellationToken), AgentError> {
        let channel = self.endpoint.connect().await?;
        let token = AppAuthServiceClient::new(channel.clone())
            .issue_token(IssueTokenRequest {
                app_id: Some(common::AppId {
                    value: self.config.app_id.clone(),
                }),
                secret: self.config.app_secret.clone(),
            })
            .await?
            .into_inner()
            .token;
        let bearer: MetadataValue<_> = format!("Bearer {token}")
            .parse()
            .map_err(|_| AgentError::InvalidToken("token is not valid header ASCII".into()))?;

        let retired = CancellationToken::new();
        let frames = futures_util::stream::unfold(
            (up_rx.clone(), retired.clone()),
            |(up_rx, retired)| async move {
                let frame = tokio::select! {
                    biased;
                    () = retired.cancelled() => None,
                    frame = async { up_rx.lock().await.recv().await } => frame,
                }?;
                Some((frame, (up_rx, retired)))
            },
        );
        let mut request = Request::new(frames);
        request.metadata_mut().insert("authorization", bearer);
        let inbound = AgentServiceClient::new(channel)
            .max_decoding_message_size(MAX_MESSAGE_BYTES)
            .max_encoding_message_size(MAX_MESSAGE_BYTES)
            .open(request)
            .await?
            .into_inner();
        Ok((inbound, retired))
    }

    /// Sends the hello, then reads the orders while the jobs run, so a cancel or the end of the
    /// stream is seen at once.
    async fn serve(
        &self,
        inbound: &mut Streaming<AgentDown>,
        jobs: &mut Jobs,
        shutdown: &CancellationToken,
    ) -> Result<Served, AgentError> {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => return Ok(Served::Shutdown),
            () = jobs.hello() => {}
        }
        loop {
            tokio::select! {
                biased;
                () = shutdown.cancelled() => return Ok(Served::Shutdown),
                job_id = jobs.finished() => debug!(%job_id, "job task ended"),
                message = inbound.message() => match message {
                    Ok(Some(down)) => self.handle(down, jobs).await,
                    Ok(None) => return Ok(Served::Closed),
                    Err(status) => {
                        warn!(error = %status, code = ?status.code(), "agent stream error");
                        let e = AgentError::Status(status);
                        return if is_terminal(&e) { Err(e) } else { Ok(Served::Closed) };
                    }
                },
            }
        }
    }

    async fn handle(&self, down: AgentDown, jobs: &mut Jobs) {
        match down.payload {
            Some(agent_down::Payload::Dispatch(dispatch)) => self.dispatch(dispatch, jobs).await,
            Some(agent_down::Payload::Cancel(cancel)) => {
                let job_id = cancel.job_id.unwrap_or_default().value;
                if jobs.cancel(&job_id) {
                    info!(%job_id, "the control plane cancelled the job");
                } else {
                    debug!(%job_id, "a cancel for a job this agent does not run");
                }
            }
            None => {}
        }
    }

    async fn dispatch(&self, dispatch: JobDispatch, jobs: &mut Jobs) {
        let job_id = dispatch.job_id.unwrap_or_default().value;
        if jobs.runs(&job_id) {
            info!(%job_id, "the job runs already; the dispatch is ignored");
            return;
        }
        info!(
            %job_id,
            pipeline_id = %dispatch.pipeline_id.unwrap_or_default().value,
            nodes = dispatch.nodes.len(),
            "received job"
        );
        let secrets = Secrets::new(
            dispatch
                .nodes
                .iter()
                .flat_map(|n| &n.env)
                .filter(|e| e.masked)
                .map(|e| e.value.clone()),
        );
        let nodes = match to_domain_nodes(dispatch.nodes) {
            Ok(nodes) => nodes,
            Err(e) => {
                warn!(%job_id, error = %e, "invalid dispatch nodes; failing the job");
                let error = format!("invalid dispatch: {e}");
                jobs.fail(job_id, CancellationToken::new(), error).await;
                return;
            }
        };
        let workspace_root = self.config.workspace_root.clone();
        let keep_workspace = self.config.keep_workspace;
        jobs.spawn(job_id, move |publisher, stop| async move {
            Executor::new(publisher, workspace_root, keep_workspace, secrets)
                .run(nodes, stop)
                .await
        });
    }
}

/// The jobs this process runs: each has its own stop token under the shutdown token. A job the
/// control plane cancels is also withdrawn: the control plane ignores its frames, so none goes out.
struct Jobs {
    root: CancellationToken,
    up: mpsc::Sender<AgentUp>,
    tasks: JoinSet<Result<(), ExecutionError>>,
    running: HashMap<task::Id, Running>,
}

struct Running {
    job_id: String,
    stop: CancellationToken,
    withdrawn: CancellationToken,
}

impl Jobs {
    fn new(root: CancellationToken, up: mpsc::Sender<AgentUp>) -> Self {
        Self {
            root,
            up,
            tasks: JoinSet::new(),
            running: HashMap::new(),
        }
    }

    fn find(&self, job_id: &str) -> Option<&Running> {
        self.running.values().find(|r| r.job_id == job_id)
    }

    fn runs(&self, job_id: &str) -> bool {
        self.find(job_id).is_some()
    }

    fn spawn<F>(
        &mut self,
        job_id: String,
        job: impl FnOnce(StatusPublisher, CancellationToken) -> F,
    ) where
        F: Future<Output = Result<(), ExecutionError>> + Send + 'static,
    {
        let running = Running {
            job_id,
            stop: self.root.child_token(),
            withdrawn: CancellationToken::new(),
        };
        let publisher = StatusPublisher::new(
            self.up.clone(),
            running.job_id.clone(),
            running.withdrawn.clone(),
        );
        let task = self.tasks.spawn(job(publisher, running.stop.clone()));
        self.running.insert(task.id(), running);
    }

    fn cancel(&self, job_id: &str) -> bool {
        self.find(job_id)
            .map(|r| {
                r.withdrawn.cancel();
                r.stop.cancel();
            })
            .is_some()
    }

    /// Resolves when a job task ends, and never while no job runs.
    async fn finished(&mut self) -> String {
        match self.tasks.join_next_with_id().await {
            Some(joined) => self.forget(joined).await,
            None => std::future::pending().await,
        }
    }

    /// A job whose executor panicked sent no terminal status; it gets one here.
    async fn forget(
        &mut self,
        joined: Result<(task::Id, Result<(), ExecutionError>), task::JoinError>,
    ) -> String {
        let task_id = joined
            .as_ref()
            .map_or_else(task::JoinError::id, |(id, _)| *id);
        let (job_id, withdrawn) = self
            .running
            .remove(&task_id)
            .map(|r| (r.job_id, r.withdrawn))
            .unwrap_or_default();
        match joined {
            Ok((_, Ok(()))) => {}
            Ok((_, Err(ExecutionError::Cancelled))) => info!(%job_id, "job stopped"),
            Ok((_, Err(e))) => error!(%job_id, error = %e, "job execution failed"),
            Err(_) => {
                error!(%job_id, "the job task panicked");
                let error = "the agent lost the job".into();
                self.fail(job_id.clone(), withdrawn, error).await;
            }
        }
        job_id
    }

    async fn fail(&self, job_id: String, withdrawn: CancellationToken, error: String) {
        let publisher = StatusPublisher::new(self.up.clone(), job_id, withdrawn);
        if let Err(e) = publisher.emit(JobEvent::JobFailed { error }).await {
            warn!(job_id = publisher.job_id(), error = %e, "failed to report the end of the job");
        }
    }

    /// Sent each time a stream opens, behind the frames that the earlier stream did not take,
    /// so a job that ended while the agent was away reports its end before this list.
    async fn hello(&mut self) {
        while let Some(joined) = self.tasks.try_join_next_with_id() {
            self.forget(joined).await;
        }
        let hello = crate::host::hello(self.running.values().map(|r| r.job_id.clone()).collect());
        info!(
            version = %hello.version,
            os = %hello.os,
            arch = %hello.arch,
            hostname = %hello.hostname,
            cpus = hello.cpu_count,
            memory_mb = hello.total_memory_mb,
            running = hello.running_jobs.len(),
            "reporting agent host to control plane"
        );
        let hello = AgentUp {
            payload: Some(agent_up::Payload::Hello(hello)),
        };
        if let Err(e) = self.up.send(hello).await {
            warn!(error = %e, "failed to report agent host; continuing without it");
        }
    }

    /// Stops every job and waits for each one to report its end.
    async fn stop(&mut self) {
        self.root.cancel();
        let ended = async {
            while let Some(joined) = self.tasks.join_next_with_id().await {
                self.forget(joined).await;
            }
        };
        if tokio::time::timeout(STOP_GRACE, ended).await.is_err() {
            warn!("a job did not stop in time; its processes are killed");
        }
    }
}

fn backoff_delay(base_secs: u64, failures: u32) -> Duration {
    let shift = failures.saturating_sub(1).min(6);
    let secs = base_secs
        .saturating_mul(1u64 << shift)
        .clamp(1, MAX_BACKOFF_SECS);
    Duration::from_secs(secs)
}

fn is_terminal(err: &AgentError) -> bool {
    matches!(err, AgentError::Status(s) if matches!(
        s.code(),
        Code::Unauthenticated | Code::PermissionDenied | Code::NotFound | Code::AlreadyExists
    ))
}

fn refusal(err: &AgentError) -> &'static str {
    match err {
        AgentError::Status(s) if s.code() == Code::Unauthenticated => {
            "the control plane refused the app credentials: unknown app, disabled app, or wrong or revoked secret"
        }
        _ => "the control plane refused the agent; not retrying",
    }
}

fn to_domain_nodes(nodes: Vec<AgentNode>) -> Result<Vec<PipelineNode>, String> {
    nodes
        .into_iter()
        .map(|n| {
            let id =
                NodeId::new(&n.node_id.unwrap_or_default().value).map_err(|e| e.to_string())?;
            let deps = n
                .deps
                .iter()
                .map(|d| NodeId::new(&d.value).map_err(|e| e.to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            let working_dir = match n.working_dir.trim() {
                "" => None,
                s => Some(WorkingDir::new(s).map_err(|e| e.to_string())?),
            };
            let env = n
                .env
                .into_iter()
                .map(|e| {
                    let key = EnvKey::new(&e.key).map_err(|err| err.to_string())?;
                    EnvVar::literal(key, e.value).map_err(|err| err.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let step = match n.step {
                Some(agent_node::Step::Exec(e)) => {
                    Step::exec(e.command, e.args).map_err(|err| err.to_string())?
                }
                Some(agent_node::Step::Script(s)) => {
                    Step::script(s.script, scylla_proto::convert::shell_from_proto(s.shell))
                        .map_err(|err| err.to_string())?
                }
                None => return Err("dispatch node is missing its step".to_string()),
            };
            Ok(PipelineNode::new(id, deps, step, working_dir, env))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use scylla_proto::agent::v1::job_status::Event;
    use scylla_proto::agent::v1::{CancelJob, ResolvedEnv};
    use scylla_proto::exec::v1::{ScriptStep, Shell};

    fn config(url: &str, root: &std::path::Path) -> AgentConfig {
        AgentConfig::parse_from([
            "scylla-agent",
            "--control-plane-url",
            url,
            "--app-id",
            "agent-1",
            "--app-secret",
            "secret",
            "--workspace-root",
            root.to_str().unwrap(),
        ])
    }

    fn jobs() -> (Jobs, mpsc::Receiver<AgentUp>) {
        let (up, frames) = mpsc::channel(64);
        (Jobs::new(CancellationToken::new(), up), frames)
    }

    fn dispatch(job_id: &str, script: &str) -> AgentDown {
        AgentDown {
            payload: Some(agent_down::Payload::Dispatch(JobDispatch {
                job_id: Some(common::JobId {
                    value: job_id.into(),
                }),
                pipeline_id: Some(common::PipelineId { value: "p".into() }),
                nodes: vec![AgentNode {
                    node_id: Some(common::NodeId { value: "n".into() }),
                    deps: vec![],
                    working_dir: String::new(),
                    env: vec![ResolvedEnv {
                        key: "A".into(),
                        value: "b".into(),
                        masked: false,
                    }],
                    step: Some(agent_node::Step::Script(ScriptStep {
                        script: script.into(),
                        shell: Shell::Sh as i32,
                    })),
                }],
            })),
        }
    }

    fn cancel(job_id: &str) -> AgentDown {
        AgentDown {
            payload: Some(agent_down::Payload::Cancel(CancelJob {
                job_id: Some(common::JobId {
                    value: job_id.into(),
                }),
            })),
        }
    }

    fn events(frames: &mut mpsc::Receiver<AgentUp>) -> Vec<String> {
        let mut events = Vec::new();
        while let Ok(frame) = frames.try_recv() {
            if let Some(agent_up::Payload::Status(status)) = frame.payload {
                assert!(status.timestamp.is_some(), "a status carries its time");
                events.push(match status.event {
                    Some(Event::JobStarted(_)) => "job_started".into(),
                    Some(Event::JobFailed(f)) => format!("job_failed:{}", f.error),
                    Some(Event::NodeStarted(_)) => "node_started".into(),
                    Some(Event::NodeSkipped(_)) => "node_skipped".into(),
                    other => format!("{other:?}"),
                });
            }
        }
        events
    }

    #[test]
    fn a_url_that_does_not_parse_fails_at_start() {
        let root = std::env::temp_dir();
        let err = Agent::new(config("not a url", &root)).err().unwrap();

        assert!(matches!(err, AgentError::InvalidUrl { .. }));
        assert!(Agent::new(config("http://127.0.0.1:1", &root)).is_ok());
    }

    #[test]
    fn a_refusal_ends_the_agent_and_an_outage_does_not() {
        let terminal = |code| is_terminal(&AgentError::Status(tonic::Status::new(code, "")));
        for code in [
            Code::Unauthenticated,
            Code::PermissionDenied,
            Code::NotFound,
            Code::AlreadyExists,
        ] {
            assert!(terminal(code), "{code:?}");
        }
        assert!(!terminal(Code::Unavailable));
        assert!(!terminal(Code::Internal));
    }

    #[tokio::test]
    async fn a_cancel_stops_only_its_job_and_stop_ends_the_rest() {
        let (mut jobs, _frames) = jobs();
        for id in ["one", "two"] {
            jobs.spawn(id.into(), |_, stop| async move {
                stop.cancelled().await;
                Ok(())
            });
        }

        assert!(jobs.cancel("one"));
        assert!(!jobs.cancel("ghost"));
        let ended = tokio::time::timeout(Duration::from_secs(5), jobs.finished())
            .await
            .unwrap();

        assert_eq!(ended, "one");
        assert!(!jobs.runs("one"));
        assert!(jobs.runs("two"));
        jobs.stop().await;
        assert!(!jobs.runs("two"));
    }

    #[tokio::test]
    async fn a_job_that_panics_still_reports_its_end() {
        let (mut jobs, mut frames) = jobs();
        jobs.spawn("boom".into(), |_, _| async { panic!("executor bug") });

        let ended = jobs.finished().await;

        assert_eq!(ended, "boom");
        assert_eq!(events(&mut frames), ["job_failed:the agent lost the job"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_cancel_from_the_control_plane_stops_the_running_job() {
        let root = std::env::temp_dir().join(format!("scylla-agent-cancel-{}", std::process::id()));
        let agent = Agent::new(config("http://127.0.0.1:1", &root)).unwrap();
        let (mut jobs, mut frames) = jobs();

        agent.handle(dispatch("job-1", "sleep 30"), &mut jobs).await;
        agent.handle(dispatch("job-1", "sleep 30"), &mut jobs).await;
        assert!(jobs.runs("job-1"));
        tokio::time::sleep(Duration::from_millis(300)).await;
        let started = Instant::now();
        agent.handle(cancel("job-1"), &mut jobs).await;
        let ended = tokio::time::timeout(Duration::from_secs(10), jobs.finished())
            .await
            .unwrap();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(ended, "job-1");
        assert!(started.elapsed() < Duration::from_secs(8));
        assert_eq!(events(&mut frames), ["job_started", "node_started"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_shutdown_fails_the_running_job_and_reports_no_node_end() {
        let root = std::env::temp_dir().join(format!("scylla-agent-stop-{}", std::process::id()));
        let agent = Agent::new(config("http://127.0.0.1:1", &root)).unwrap();
        let (mut jobs, mut frames) = jobs();

        agent.handle(dispatch("job-3", "sleep 30"), &mut jobs).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        jobs.stop().await;
        let _ = std::fs::remove_dir_all(&root);

        assert!(!jobs.runs("job-3"));
        assert_eq!(
            events(&mut frames),
            [
                "job_started",
                "node_started",
                "job_failed:job failed: execution cancelled",
            ]
        );
    }

    #[tokio::test]
    async fn an_invalid_dispatch_fails_its_job_without_running_it() {
        let root = std::env::temp_dir();
        let agent = Agent::new(config("http://127.0.0.1:1", &root)).unwrap();
        let (mut jobs, mut frames) = jobs();
        let mut invalid = dispatch("job-2", "echo hi");
        if let Some(agent_down::Payload::Dispatch(d)) = &mut invalid.payload {
            d.nodes[0].step = None;
        }

        agent.handle(invalid, &mut jobs).await;

        assert!(!jobs.runs("job-2"));
        let events = events(&mut frames);
        assert_eq!(events.len(), 1);
        assert!(
            events[0].starts_with("job_failed:invalid dispatch"),
            "{events:?}"
        );
    }
}
