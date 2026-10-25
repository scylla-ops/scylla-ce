//! The agent against a control plane that the test drives, over a real gRPC stream.

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Parser;
use scylla_agent::{Agent, AgentConfig};
use scylla_proto::agent::v1::agent_service_server::{AgentService, AgentServiceServer};
use scylla_proto::agent::v1::job_status::Event;
use scylla_proto::agent::v1::{
    AgentDown, AgentNode, AgentUp, CancelJob, JobDispatch, agent_down, agent_node, agent_up,
};
use scylla_proto::app::v1::app_auth_service_server::{AppAuthService, AppAuthServiceServer};
use scylla_proto::app::v1::{IssueTokenRequest, IssueTokenResponse};
use scylla_proto::common::v1 as common;
use scylla_proto::exec::v1::{ScriptStep, Shell};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_stream::Stream;
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status, Streaming};

type Orders = mpsc::Sender<Result<AgentDown, Status>>;

/// What the agent sent, stream by stream.
#[derive(Debug, Clone, PartialEq)]
enum Frame {
    Hello(Vec<String>),
    Status(String, &'static str),
    End,
}

#[derive(Clone, Default)]
struct ControlPlane {
    frames: Arc<Mutex<Vec<(usize, Frame)>>>,
    streams: Arc<Mutex<Vec<Option<Orders>>>>,
}

impl ControlPlane {
    fn frames(&self) -> Vec<(usize, Frame)> {
        self.frames.lock().unwrap().clone()
    }

    fn has(&self, frame: &Frame) -> bool {
        self.frames().iter().any(|(_, f)| f == frame)
    }

    async fn wait_for(&self, stream: usize, frame: Frame) {
        let frame = (stream, frame);
        for _ in 0..400 {
            if self.frames().contains(&frame) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("no {frame:?} in {:?}", self.frames());
    }

    async fn send(&self, stream: usize, order: agent_down::Payload) {
        let orders = self.streams.lock().unwrap()[stream].clone().unwrap();
        orders
            .send(Ok(AgentDown {
                payload: Some(order),
            }))
            .await
            .unwrap();
    }

    fn close(&self, stream: usize) {
        self.streams.lock().unwrap()[stream] = None;
    }
}

fn label(event: &Event) -> &'static str {
    match event {
        Event::JobStarted(_) => "job_started",
        Event::JobCompleted(_) => "job_completed",
        Event::JobFailed(_) => "job_failed",
        Event::NodeStarted(_) => "node_started",
        Event::NodeCompleted(_) => "node_completed",
        Event::NodeFailed(_) => "node_failed",
        Event::NodeSkipped(_) => "node_skipped",
        Event::NodeRetrying(_) => "node_retrying",
    }
}

#[tonic::async_trait]
impl AppAuthService for ControlPlane {
    async fn issue_token(
        &self,
        _: Request<IssueTokenRequest>,
    ) -> Result<Response<IssueTokenResponse>, Status> {
        Ok(Response::new(IssueTokenResponse {
            token: "token".into(),
            expires_at: None,
        }))
    }
}

#[tonic::async_trait]
impl AgentService for ControlPlane {
    type OpenStream = Pin<Box<dyn Stream<Item = Result<AgentDown, Status>> + Send + 'static>>;

    async fn open(
        &self,
        request: Request<Streaming<AgentUp>>,
    ) -> Result<Response<Self::OpenStream>, Status> {
        let (orders, received) = mpsc::channel(16);
        let stream = {
            let mut streams = self.streams.lock().unwrap();
            streams.push(Some(orders));
            streams.len() - 1
        };
        let mut inbound = request.into_inner();
        let plane = self.clone();
        tokio::spawn(async move {
            while let Ok(Some(up)) = inbound.message().await {
                let frame = match up.payload {
                    Some(agent_up::Payload::Hello(hello)) => {
                        Frame::Hello(hello.running_jobs.into_iter().map(|j| j.value).collect())
                    }
                    Some(agent_up::Payload::Status(status)) => Frame::Status(
                        status.job_id.unwrap_or_default().value,
                        status.event.as_ref().map_or("none", label),
                    ),
                    _ => continue,
                };
                plane.frames.lock().unwrap().push((stream, frame));
            }
            plane.frames.lock().unwrap().push((stream, Frame::End));
            plane.close(stream);
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(received))))
    }
}

async fn control_plane() -> (ControlPlane, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let plane = ControlPlane::default();
    let server = tonic::transport::Server::builder()
        .add_service(AppAuthServiceServer::new(plane.clone()))
        .add_service(AgentServiceServer::new(plane.clone()))
        .serve_with_incoming(TcpListenerStream::new(listener));
    tokio::spawn(server);
    (plane, url)
}

fn start_agent(url: &str, tag: &str) -> (CancellationToken, tokio::task::JoinHandle<()>) {
    let root = std::env::temp_dir().join(format!("scylla-agent-{tag}-{}", std::process::id()));
    let config = AgentConfig::parse_from([
        "scylla-agent",
        "--control-plane-url",
        url,
        "--app-id",
        "agent-1",
        "--app-secret",
        "secret",
        "--reconnect-backoff-secs",
        "1",
        "--workspace-root",
        root.to_str().unwrap(),
    ]);
    let shutdown = CancellationToken::new();
    let stop = shutdown.clone();
    let task = tokio::spawn(async move {
        Agent::new(config).unwrap().run(stop).await.unwrap();
        let _ = std::fs::remove_dir_all(root);
    });
    (shutdown, task)
}

fn dispatch(job_id: &str, script: &str) -> agent_down::Payload {
    attempts(job_id, script, 0, 0)
}

fn attempts(job_id: &str, script: &str, timeout_seconds: u32, retries: u32) -> agent_down::Payload {
    agent_down::Payload::Dispatch(JobDispatch {
        job_id: Some(common::JobId {
            value: job_id.into(),
        }),
        pipeline_id: Some(common::PipelineId { value: "p".into() }),
        nodes: vec![AgentNode {
            node_id: Some(common::NodeId { value: "n".into() }),
            deps: vec![],
            working_dir: String::new(),
            env: vec![],
            timeout_seconds,
            retries,
            step: Some(agent_node::Step::Script(ScriptStep {
                script: script.into(),
                shell: Shell::Sh as i32,
            })),
        }],
    })
}

fn count(plane: &ControlPlane, frame: &Frame) -> usize {
    plane.frames().iter().filter(|(_, f)| f == frame).count()
}

fn cancel(job_id: &str) -> agent_down::Payload {
    agent_down::Payload::Cancel(CancelJob {
        job_id: Some(common::JobId {
            value: job_id.into(),
        }),
    })
}

fn status(job_id: &str, event: &'static str) -> Frame {
    Frame::Status(job_id.into(), event)
}

#[tokio::test]
async fn a_cancel_stops_a_job_and_a_shutdown_stops_the_rest_before_the_stream_closes() {
    let (plane, url) = control_plane().await;
    let (shutdown, agent) = start_agent(&url, "cancel");
    plane.wait_for(0, Frame::Hello(vec![])).await;

    plane.send(0, dispatch("job-1", "sleep 30")).await;
    plane.wait_for(0, status("job-1", "node_started")).await;
    plane.send(0, cancel("job-1")).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    plane.close(0);
    plane.wait_for(1, Frame::Hello(vec![])).await;

    plane.send(1, dispatch("job-2", "sleep 30")).await;
    plane.wait_for(1, status("job-2", "node_started")).await;
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(11), agent)
        .await
        .expect("the agent stops within its bound")
        .unwrap();

    let frames = plane.frames();
    let failed = frames
        .iter()
        .position(|(_, f)| f == &status("job-2", "job_failed"))
        .expect("the stopped job reports its end");
    let end = frames
        .iter()
        .position(|frame| frame == &(1, Frame::End))
        .expect("the agent closes its stream");
    assert!(failed < end, "{frames:?}");
    assert!(!plane.has(&status("job-1", "job_completed")));
    assert!(!plane.has(&status("job-1", "job_failed")), "{frames:?}");
}

#[tokio::test]
async fn a_job_runs_on_across_a_lost_stream_and_reports_on_the_next_one() {
    let (plane, url) = control_plane().await;
    let (shutdown, agent) = start_agent(&url, "reconnect");
    plane.wait_for(0, Frame::Hello(vec![])).await;

    plane.send(0, dispatch("job-1", "sleep 3; echo done")).await;
    plane.wait_for(0, status("job-1", "node_started")).await;
    plane.close(0);

    plane.wait_for(1, Frame::Hello(vec!["job-1".into()])).await;
    plane.wait_for(1, status("job-1", "job_completed")).await;
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(11), agent)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn a_node_that_fails_twice_then_passes_completes_on_its_third_attempt() {
    let (plane, url) = control_plane().await;
    let (shutdown, agent) = start_agent(&url, "retry");
    plane.wait_for(0, Frame::Hello(vec![])).await;

    let script =
        "n=$(cat tries 2>/dev/null || echo 0); n=$((n + 1)); echo $n > tries; [ $n -ge 3 ]";
    plane.send(0, attempts("job-1", script, 0, 2)).await;
    plane.wait_for(0, status("job-1", "job_completed")).await;
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(11), agent)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(count(&plane, &status("job-1", "node_started")), 1);
    assert_eq!(count(&plane, &status("job-1", "node_retrying")), 2);
    assert_eq!(count(&plane, &status("job-1", "node_completed")), 1);
    assert!(!plane.has(&status("job-1", "node_failed")));
}

#[tokio::test]
async fn a_node_that_passes_its_timeout_fails_before_its_step_ends() {
    let (plane, url) = control_plane().await;
    let (shutdown, agent) = start_agent(&url, "timeout");
    plane.wait_for(0, Frame::Hello(vec![])).await;

    let started = std::time::Instant::now();
    plane.send(0, attempts("job-1", "sleep 10", 1, 0)).await;
    plane.wait_for(0, status("job-1", "job_failed")).await;
    let elapsed = started.elapsed();
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(11), agent)
        .await
        .unwrap()
        .unwrap();

    assert!(elapsed < Duration::from_secs(10), "took {elapsed:?}");
    assert!(plane.has(&status("job-1", "node_failed")));
    assert!(!plane.has(&status("job-1", "node_retrying")));
}

#[tokio::test]
async fn a_cancel_during_a_retry_starts_no_new_attempt() {
    let (plane, url) = control_plane().await;
    let (shutdown, agent) = start_agent(&url, "retry-cancel");
    plane.wait_for(0, Frame::Hello(vec![])).await;
    let marks = std::env::temp_dir().join(format!(
        "scylla-agent-retry-cancel-{}.marks",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&marks);

    let script = format!(
        "echo attempt >> '{}'; [ \"$(wc -l < '{}')\" -ge 2 ] || exit 1; sleep 30",
        marks.display(),
        marks.display()
    );
    plane.send(0, attempts("job-1", &script, 0, 5)).await;
    plane.wait_for(0, status("job-1", "node_retrying")).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    plane.send(0, cancel("job-1")).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(11), agent)
        .await
        .unwrap()
        .unwrap();
    let attempts_run = std::fs::read_to_string(&marks).unwrap().lines().count();
    let _ = std::fs::remove_file(&marks);

    assert_eq!(attempts_run, 2);
    assert_eq!(count(&plane, &status("job-1", "node_retrying")), 1);
    assert!(!plane.has(&status("job-1", "job_completed")));
}
