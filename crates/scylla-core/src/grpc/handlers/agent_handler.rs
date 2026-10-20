use crate::application::actions::app_only;
use crate::application::agent::{AgentOrder, AgentStream, RecordAgentHost, TouchAgent};
use crate::application::job::{
    AppendJobLogs, ReconcileAgentJobs, RecordJobStatus, ReleaseAgentJobs,
};
use crate::application::{AgentUseCases, JobDispatch, JobLogUseCases, JobUseCases};
use crate::domain::agent::AgentHost;
use crate::domain::caller::CallerContext;
use crate::domain::clock;
use crate::domain::errors::DomainResult;
use crate::domain::ids::JobId;
use crate::domain::job::JobLog;
use crate::domain::pipeline::{NodeId, Step};
use crate::extract_auth_context;
use crate::grpc::convert::dt;
use crate::grpc::mappers::domain_error_to_status;
use crate::infrastructure::InMemoryAgentRegistry;
use crate::infrastructure::messaging::agent_registry::ConnHandle;
use derive_more::Constructor;
use futures_util::{StreamExt, future, stream};
use scylla_extension::{Actions, Kind, Path};
use scylla_proto::agent::v1::{
    AgentDown, AgentHello, AgentNode, AgentUp, CancelJob, JobDispatch as ProtoJobDispatch,
    JobLogLine, JobStatus, ResolvedEnv, agent_down, agent_node, agent_service_server::AgentService,
    agent_up,
};
use scylla_proto::common::v1 as common;
use scylla_proto::exec::v1 as exec;
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status, Streaming};
use tracing::{debug, warn};

/// The frames the reader takes at once: the lines among them go to the store in one batch.
const READ_CHUNK: usize = 512;

#[derive(Clone, Constructor)]
pub struct AgentHandler {
    actions: Arc<Actions>,
    jobs: Arc<JobUseCases>,
    logs: Arc<JobLogUseCases>,
    agents: Arc<AgentUseCases>,
    registry: Arc<InMemoryAgentRegistry>,
}

#[async_trait::async_trait]
impl AgentService for AgentHandler {
    type OpenStream = Pin<Box<dyn Stream<Item = Result<AgentDown, Status>> + Send + 'static>>;

    async fn open(
        &self,
        request: Request<Streaming<AgentUp>>,
    ) -> Result<Response<Self::OpenStream>, Status> {
        let caller = caller!(request);
        self.actions
            .run(&*self.agents, &caller, TouchAgent)
            .await
            .map_err(domain_error_to_status)?;
        let agent = app_only(&caller).map_err(domain_error_to_status)?;

        let conn = self.registry.register(&agent);
        let reader = Reader {
            handler: self.clone(),
            caller,
            stream: conn.stream.clone(),
            stop: conn.stop.clone(),
        };
        tokio::spawn(reader.read(request.into_inner()));
        Ok(Response::new(Box::pin(down_stream(conn))))
    }
}

struct Reader {
    handler: AgentHandler,
    caller: CallerContext,
    stream: AgentStream,
    stop: CancellationToken,
}

impl Reader {
    async fn run<A: Path<K, R>, K: Kind, R>(
        &self,
        runner: &R,
        action: A,
    ) -> DomainResult<A::Output> {
        self.handler.actions.run(runner, &self.caller, action).await
    }

    /// Reads until the agent closes its side, a frame does not decode, or the registry drops the
    /// stream, which also drops the rest of the chunk in hand. Then the jobs placed on this
    /// stream that have not started go back to the pool: a newer stream of the agent keeps its
    /// own.
    async fn read(self, inbound: Streaming<AgentUp>) {
        let stopped = self.stop.clone().cancelled_owned();
        let mut chunks = pin!(inbound.take_until(stopped).ready_chunks(READ_CHUNK));
        'read: while let Some(chunk) = chunks.next().await {
            for item in work(chunk) {
                if self.stop.is_cancelled() {
                    break 'read;
                }
                match item {
                    Work::Logs(job_id, logs) => self.append(job_id, logs).await,
                    Work::Status(status) => self.record(status).await,
                    Work::Hello(hello) => self.hello(hello).await,
                    Work::Undecodable(status) => {
                        warn!(app_id = %self.stream.agent, error = %status, "an agent frame does not decode; closing the stream");
                        break 'read;
                    }
                }
            }
        }
        self.touch().await;
        self.handler.registry.unregister(&self.stream);
        let release = ReleaseAgentJobs {
            stream: self.stream.id.clone(),
        };
        if let Err(e) = self.run(&*self.handler.jobs, release).await {
            warn!(app_id = %self.stream.agent, error = %e, "failed to release the jobs placed on the stream");
        }
    }

    async fn touch(&self) {
        if let Err(e) = self.run(&*self.handler.agents, TouchAgent).await {
            warn!(app_id = %self.stream.agent, error = %e, "failed to update agent last_seen");
        }
    }

    async fn record(&self, status: JobStatus) {
        let job_id = JobId::new(status.job_id.clone().unwrap_or_default().value);
        if let Some(event) = scylla_proto::convert::status_to_job_event(&status) {
            let record = RecordJobStatus {
                job_id: job_id.clone(),
                event,
                at: dt(status.timestamp).unwrap_or_else(clock::now),
            };
            if let Err(e) = self.run(&*self.handler.jobs, record).await {
                debug!(app_id = %self.stream.agent, job_id = %job_id, error = %e, "refused a job status");
            }
        }
        self.touch().await;
    }

    async fn append(&self, job_id: JobId, logs: Vec<JobLog>) {
        let append = AppendJobLogs {
            job_id: job_id.clone(),
            logs,
        };
        if let Err(e) = self.run(&*self.handler.logs, append).await {
            debug!(app_id = %self.stream.agent, job_id = %job_id, error = %e, "refused job log lines");
        }
    }

    async fn hello(&self, hello: AgentHello) {
        let host = RecordAgentHost {
            host: hello_to_domain(&hello),
        };
        if let Err(e) = self.run(&*self.handler.agents, host).await {
            warn!(app_id = %self.stream.agent, error = %e, "failed to record agent host");
        }
        let reconcile = ReconcileAgentJobs {
            running: hello
                .running_jobs
                .into_iter()
                .map(|id| JobId::new(id.value))
                .collect(),
        };
        if let Err(e) = self.run(&*self.handler.jobs, reconcile).await {
            warn!(app_id = %self.stream.agent, error = %e, "failed to reconcile the jobs of the agent");
        }
        self.touch().await;
    }
}

#[derive(Debug)]
enum Work {
    Logs(JobId, Vec<JobLog>),
    Status(JobStatus),
    Hello(AgentHello),
    Undecodable(Status),
}

/// The frames of one chunk as work, in order. The lines that come between two other frames
/// form one batch for each job, so a status never overtakes a line sent before it.
fn work(chunk: Vec<Result<AgentUp, Status>>) -> Vec<Work> {
    fn flush(work: &mut Vec<Work>, lines: &mut Vec<(JobId, Vec<JobLog>)>) {
        work.extend(
            lines
                .drain(..)
                .map(|(job_id, logs)| Work::Logs(job_id, logs)),
        );
    }
    let mut work = Vec::new();
    let mut lines: Vec<(JobId, Vec<JobLog>)> = Vec::new();
    for frame in chunk {
        let payload = match frame {
            Ok(up) => up.payload,
            Err(status) => {
                flush(&mut work, &mut lines);
                work.push(Work::Undecodable(status));
                return work;
            }
        };
        match payload {
            Some(agent_up::Payload::Log(line)) => {
                if let Some(log) = log_line_to_domain(&line) {
                    match lines.iter_mut().find(|(job_id, _)| job_id == log.job_id()) {
                        Some((_, logs)) => logs.push(log),
                        None => lines.push((log.job_id().clone(), vec![log])),
                    }
                }
            }
            Some(agent_up::Payload::Status(status)) => {
                flush(&mut work, &mut lines);
                work.push(Work::Status(status));
            }
            Some(agent_up::Payload::Hello(hello)) => {
                flush(&mut work, &mut lines);
                work.push(Work::Hello(hello));
            }
            None => {}
        }
    }
    flush(&mut work, &mut lines);
    work
}

/// The orders of one connection. The registry ends it by dropping the connection: after a
/// disconnect it ends, and when a newer stream of the same app replaced it, it ends with
/// `ALREADY_EXISTS`. It checks the stop before the orders, so no order that waits in the queue
/// goes out after a stop. Its drop stops the reader.
fn down_stream(conn: ConnHandle) -> impl Stream<Item = Result<AgentDown, Status>> + Send {
    let stopped = conn.stop.clone().cancelled_owned();
    let stop_reader = conn.stop.drop_guard();
    let replaced = conn.replaced;
    let end = async move {
        drop(stop_reader);
        replaced.load(Ordering::Relaxed).then(|| {
            Err(Status::already_exists(
                "another agent process opened a stream with this app",
            ))
        })
    };
    ReceiverStream::new(conn.orders)
        .map(|order| Ok(order_to_proto(order)))
        .take_until(stopped)
        .chain(stream::once(end).filter_map(future::ready))
}

fn hello_to_domain(hello: &AgentHello) -> AgentHost {
    AgentHost {
        version: hello.version.clone(),
        os: hello.os.clone(),
        arch: hello.arch.clone(),
        hostname: hello.hostname.clone(),
        cpu_count: (hello.cpu_count > 0).then_some(hello.cpu_count),
        total_memory_mb: (hello.total_memory_mb > 0).then_some(hello.total_memory_mb),
        reported_at: clock::now(),
    }
}

fn order_to_proto(order: AgentOrder) -> AgentDown {
    let payload = match order {
        AgentOrder::Run(dispatch) => agent_down::Payload::Dispatch(dispatch_to_proto(&dispatch)),
        AgentOrder::Cancel(job_id) => agent_down::Payload::Cancel(CancelJob {
            job_id: Some(common::JobId {
                value: job_id.to_string(),
            }),
        }),
    };
    AgentDown {
        payload: Some(payload),
    }
}

fn dispatch_to_proto(dispatch: &JobDispatch) -> ProtoJobDispatch {
    let nodes = dispatch
        .nodes
        .iter()
        .map(|n| AgentNode {
            node_id: Some(common::NodeId {
                value: n.id.clone(),
            }),
            deps: n
                .deps
                .iter()
                .map(|d| common::NodeId { value: d.clone() })
                .collect(),
            working_dir: n.working_dir.clone().unwrap_or_default(),
            env: n
                .env
                .iter()
                .map(|ev| ResolvedEnv {
                    key: ev.key.clone(),
                    value: ev.value.clone(),
                    masked: ev.masked,
                })
                .collect(),
            step: Some(step_to_proto(&n.step)),
        })
        .collect();
    ProtoJobDispatch {
        job_id: Some(common::JobId {
            value: dispatch.job_id.clone(),
        }),
        pipeline_id: Some(common::PipelineId {
            value: dispatch.pipeline_id.clone(),
        }),
        nodes,
    }
}

fn step_to_proto(step: &Step) -> agent_node::Step {
    match step {
        Step::Exec { command, args } => agent_node::Step::Exec(exec::ExecStep {
            command: command.to_string(),
            args: args.iter().map(ToString::to_string).collect(),
        }),
        Step::Script { script, shell } => agent_node::Step::Script(exec::ScriptStep {
            script: script.to_string(),
            shell: scylla_proto::convert::shell_to_proto(*shell) as i32,
        }),
    }
}

fn log_line_to_domain(line: &JobLogLine) -> Option<JobLog> {
    let node_id_str = line.node_id.clone().unwrap_or_default().value;
    let node_id = NodeId::new(&node_id_str)
        .map_err(|e| warn!(node_id = %node_id_str, error = %e, "invalid node_id in agent log"))
        .ok()?;
    let stream = scylla_proto::convert::log_stream_from_proto(line.stream);
    let timestamp = dt(line.timestamp).unwrap_or_else(clock::now);
    Some(JobLog::new(
        JobId::new(line.job_id.clone().unwrap_or_default().value),
        node_id,
        stream,
        line.line.clone(),
        timestamp,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::agent::AgentDispatch;
    use crate::domain::ids::AppId;
    use scylla_proto::agent::v1::job_status::{Event, JobStarted};
    use tokio::sync::mpsc;

    fn line(job: &str, text: &str) -> Result<AgentUp, Status> {
        Ok(AgentUp {
            payload: Some(agent_up::Payload::Log(JobLogLine {
                job_id: Some(common::JobId { value: job.into() }),
                node_id: Some(common::NodeId { value: "n".into() }),
                stream: 0,
                line: text.into(),
                timestamp: None,
            })),
        })
    }

    fn started(job: &str) -> Result<AgentUp, Status> {
        Ok(AgentUp {
            payload: Some(agent_up::Payload::Status(JobStatus {
                job_id: Some(common::JobId { value: job.into() }),
                timestamp: None,
                event: Some(Event::JobStarted(JobStarted {})),
            })),
        })
    }

    fn shape(work: &[Work]) -> Vec<String> {
        work.iter()
            .map(|w| match w {
                Work::Logs(job, logs) => format!(
                    "{job}:{}",
                    logs.iter().map(JobLog::line).collect::<Vec<_>>().join(",")
                ),
                Work::Status(_) => "status".into(),
                Work::Hello(_) => "hello".into(),
                Work::Undecodable(_) => "undecodable".into(),
            })
            .collect()
    }

    #[test]
    fn lines_before_a_status_are_stored_before_it_in_one_batch_per_job() {
        let chunk = vec![
            line("a", "1"),
            line("b", "1"),
            line("a", "2"),
            started("a"),
            line("a", "3"),
        ];

        assert_eq!(shape(&work(chunk)), ["a:1,2", "b:1", "status", "a:3"]);
    }

    #[test]
    fn an_undecodable_frame_ends_the_work_after_the_lines_before_it() {
        let chunk = vec![
            line("a", "1"),
            Err(Status::out_of_range("too large")),
            line("a", "2"),
        ];

        assert_eq!(shape(&work(chunk)), ["a:1", "undecodable"]);
    }

    fn registry() -> Arc<InMemoryAgentRegistry> {
        Arc::new(InMemoryAgentRegistry::new(mpsc::unbounded_channel().0))
    }

    #[tokio::test]
    async fn a_replaced_stream_ends_with_already_exists() {
        let registry = registry();
        let app = AppId::new("agent-1");
        let mut first = Box::pin(down_stream(registry.register(&app)));
        let _second = registry.register(&app);

        let err = first.next().await.unwrap().unwrap_err();

        assert_eq!(err.code(), tonic::Code::AlreadyExists);
        assert!(first.next().await.is_none());
    }

    #[tokio::test]
    async fn a_disconnected_stream_ends_and_drops_the_orders_that_wait() {
        let registry = registry();
        let app = AppId::new("agent-1");
        let mut stream = Box::pin(down_stream(registry.register(&app)));
        registry.cancel(&app, &JobId::new("job-1")).unwrap();

        registry.disconnect(&app);

        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn an_order_goes_out_as_its_frame() {
        let registry = registry();
        let app = AppId::new("agent-1");
        let mut stream = Box::pin(down_stream(registry.register(&app)));
        registry.cancel(&app, &JobId::new("job-1")).unwrap();

        let frame = stream.next().await.unwrap().unwrap();

        assert!(matches!(
            frame.payload,
            Some(agent_down::Payload::Cancel(CancelJob { job_id: Some(id) })) if id.value == "job-1"
        ));
    }

    #[tokio::test]
    async fn dropping_the_stream_stops_the_reader() {
        let registry = registry();
        let conn = registry.register(&AppId::new("agent-1"));
        let stop = conn.stop.clone();

        drop(down_stream(conn));

        assert!(stop.is_cancelled());
    }

    mod stream {
        use super::*;
        use crate::application::agent::DispatchPendingJobs;
        use crate::application::agent::{AgentRepository, AgentStats};
        use crate::application::pagination::{PaginatedResult, PaginationParams};
        use crate::application::{
            AppRepository, DispatchUseCases, JobLogRepository, PrincipalAccess,
        };
        use crate::domain::agent::Agent;
        use crate::domain::app::{App, AppCredential};
        use crate::domain::caller::ServiceIdentity;
        use crate::domain::ids::{OrganizationId, ProjectId};
        use crate::domain::job::{Job, JobStatus as Phase};
        use crate::grpc::middleware::auth_interceptor::AuthContext;
        use crate::test_support::authz::{RecordingPermissionService, actions};
        use crate::test_support::jobs::JobBuilder;
        use crate::test_support::pipelines::PipelineBuilder;
        use crate::test_support::stubs::{StubJobs, StubTails, alice, dispatcher, empty_page};
        use chrono::{DateTime, Utc};
        use scylla_auth::authz::Grant;
        use scylla_proto::agent::v1::agent_service_client::AgentServiceClient;
        use scylla_proto::agent::v1::agent_service_server::AgentServiceServer;
        use scylla_proto::agent::v1::job_status::JobCompleted;
        use std::sync::Mutex;
        use std::time::Duration;
        use tokio::net::TcpListener;
        use tokio_stream::wrappers::TcpListenerStream;

        /// Counts the touches: one when the stream opens, one after each hello.
        #[derive(Default)]
        struct Agents(Mutex<usize>);

        #[async_trait::async_trait]
        impl AgentRepository for Agents {
            async fn find_by_app_id(&self, _: &AppId) -> DomainResult<Agent> {
                unreachable!("the stream reads no agent row")
            }
            async fn list_by_organization(&self, _: &OrganizationId) -> DomainResult<Vec<Agent>> {
                unreachable!("the stream lists no agent")
            }
            async fn touch_last_seen(&self, _: &AppId, _: DateTime<Utc>) -> DomainResult<()> {
                *self.0.lock().unwrap() += 1;
                Ok(())
            }
            async fn record_host(&self, _: &AppId, _: &AgentHost) -> DomainResult<()> {
                Ok(())
            }
            async fn agent_stats(&self, _: &AppId) -> DomainResult<AgentStats> {
                unreachable!("the stream reads no stats")
            }
        }

        struct Apps;

        #[async_trait::async_trait]
        impl AppRepository for Apps {
            async fn create_app(&self, _: &App, _: &AppCredential) -> DomainResult<()> {
                unreachable!("the stream writes no app")
            }
            async fn provision_agent(
                &self,
                _: &App,
                _: &AppCredential,
                _: &Agent,
                _: &Grant,
            ) -> DomainResult<()> {
                unreachable!("the stream writes no app")
            }
            async fn provision(&self, _: &App, _: &Grant) -> DomainResult<()> {
                unreachable!("the stream writes no app")
            }
            async fn find_by_id(&self, _: &AppId) -> DomainResult<App> {
                unreachable!("the stream reads no app")
            }
            async fn find_trigger_runner(&self, _: &OrganizationId) -> DomainResult<Option<AppId>> {
                unreachable!("the stream reads no app")
            }
            async fn list_by_organization(&self, _: &OrganizationId) -> DomainResult<Vec<App>> {
                unreachable!("the stream reads no app")
            }
            async fn set_active(&self, _: &AppId, _: bool) -> DomainResult<()> {
                unreachable!("the stream writes no app")
            }
            async fn delete(&self, _: &AppId) -> DomainResult<()> {
                unreachable!("the stream writes no app")
            }
            async fn list_access(&self, _: &AppId) -> DomainResult<Vec<PrincipalAccess>> {
                unreachable!("the stream reads no app access")
            }
        }

        #[derive(Default)]
        struct Lines(Mutex<Vec<JobLog>>);

        #[async_trait::async_trait]
        impl JobLogRepository for Lines {
            async fn create_many(&self, logs: &[JobLog]) -> DomainResult<()> {
                self.0.lock().unwrap().extend_from_slice(logs);
                Ok(())
            }
            async fn list_by_job(
                &self,
                _: &JobId,
                _: Option<&PaginationParams>,
            ) -> DomainResult<PaginatedResult<JobLog>> {
                empty_page()
            }
            async fn list_by_job_and_node(
                &self,
                _: &JobId,
                _: &NodeId,
                _: Option<&PaginationParams>,
            ) -> DomainResult<PaginatedResult<JobLog>> {
                empty_page()
            }
            async fn list_all_by_job(
                &self,
                _: &JobId,
                _: Option<&NodeId>,
            ) -> DomainResult<Vec<JobLog>> {
                Ok(Vec::new())
            }
        }

        struct Lab {
            url: String,
            actions: Arc<Actions>,
            jobs: Arc<StubJobs>,
            job_uc: Arc<JobUseCases>,
            dispatch: Arc<DispatchUseCases>,
            agents: Arc<Agents>,
            lines: Arc<Lines>,
            registry: Arc<InMemoryAgentRegistry>,
        }

        /// The agent's side of one stream: the frames it sends and the orders it gets.
        struct Agent1 {
            frames: mpsc::Sender<AgentUp>,
            orders: Streaming<AgentDown>,
        }

        impl Agent1 {
            async fn send(&self, payload: agent_up::Payload) {
                self.frames
                    .send(AgentUp {
                        payload: Some(payload),
                    })
                    .await
                    .unwrap();
            }

            async fn hello(&self, running: &[&Job]) {
                self.send(agent_up::Payload::Hello(AgentHello {
                    running_jobs: running
                        .iter()
                        .map(|job| common::JobId {
                            value: job.id().to_string(),
                        })
                        .collect(),
                    ..AgentHello::default()
                }))
                .await;
            }

            /// The next order, or `None` once the control plane ends the stream.
            async fn order(&mut self) -> Option<agent_down::Payload> {
                tokio::time::timeout(Duration::from_secs(5), self.orders.message())
                    .await
                    .unwrap()
                    .unwrap()
                    .map(|down| down.payload.unwrap())
            }
        }

        fn agent_id() -> AppId {
            AppId::new("agent-1")
        }

        async fn lab(jobs: Vec<Job>) -> Lab {
            let actions = Arc::new(actions(Arc::new(RecordingPermissionService::new())));
            let registry = Arc::new(InMemoryAgentRegistry::new(mpsc::unbounded_channel().0));
            let jobs = Arc::new(StubJobs::with(jobs));
            let tails = Arc::new(StubTails::default());
            let dispatch = dispatcher(registry.clone(), jobs.clone(), tails.clone());
            let job_uc = Arc::new(JobUseCases::new(
                jobs.clone(),
                tails.clone(),
                dispatch.clone(),
            ));
            let agents = Arc::new(Agents::default());
            let lines = Arc::new(Lines::default());
            let handler = AgentHandler::new(
                actions.clone(),
                job_uc.clone(),
                Arc::new(JobLogUseCases::new(lines.clone(), tails, jobs.clone())),
                Arc::new(AgentUseCases::new(
                    Arc::new(Apps),
                    agents.clone(),
                    jobs.clone(),
                    Arc::new(crate::test_support::stubs::StubHash::secrets()),
                    registry.clone(),
                )),
                registry.clone(),
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let service =
                AgentServiceServer::with_interceptor(handler, |mut request: Request<()>| {
                    request.extensions_mut().insert(AuthContext {
                        caller: CallerContext::App(agent_id()),
                    });
                    Ok(request)
                });
            tokio::spawn(
                tonic::transport::Server::builder()
                    .add_service(service)
                    .serve_with_incoming(TcpListenerStream::new(listener)),
            );
            Lab {
                url,
                actions,
                jobs,
                job_uc,
                dispatch,
                agents,
                lines,
                registry,
            }
        }

        impl Lab {
            async fn open(&self) -> Agent1 {
                let (frames, outgoing) = mpsc::channel(16);
                let mut client = AgentServiceClient::connect(self.url.clone()).await.unwrap();
                let orders = client
                    .open(ReceiverStream::new(outgoing))
                    .await
                    .unwrap()
                    .into_inner();
                Agent1 { frames, orders }
            }

            fn touches(&self) -> usize {
                *self.agents.0.lock().unwrap()
            }
        }

        async fn until(check: impl Fn() -> bool) {
            for _ in 0..200 {
                if check() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("the condition never held");
        }

        fn cancelled(order: Option<agent_down::Payload>) -> String {
            match order {
                Some(agent_down::Payload::Cancel(CancelJob { job_id: Some(id) })) => id.value,
                other => panic!("expected a cancel, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn a_stream_reconciles_its_jobs_takes_a_cancel_and_ends_on_a_disconnect() {
            let pipeline = PipelineBuilder::for_project_id(ProjectId::new("p")).build();
            let running = || {
                JobBuilder::new(&pipeline)
                    .running(true)
                    .agent(agent_id())
                    .build()
            };
            let (kept, lost) = (running(), running());
            let lab = lab(vec![kept.clone(), lost.clone()]).await;
            let mut agent = lab.open().await;

            agent.hello(&[&kept]).await;
            until(|| lab.jobs.row(lost.id()).status() == Phase::Orphaned).await;
            assert_eq!(lab.jobs.row(kept.id()).status(), Phase::Running);
            lab.actions
                .run(
                    &*lab.job_uc,
                    &alice(),
                    crate::application::job::CancelJob {
                        id: kept.id().clone(),
                    },
                )
                .await
                .unwrap();
            assert_eq!(cancelled(agent.order().await), lost.id().to_string());
            assert_eq!(cancelled(agent.order().await), kept.id().to_string());

            agent
                .send(agent_up::Payload::Status(JobStatus {
                    job_id: Some(common::JobId {
                        value: kept.id().to_string(),
                    }),
                    timestamp: None,
                    event: Some(scylla_proto::agent::v1::job_status::Event::JobCompleted(
                        JobCompleted {},
                    )),
                }))
                .await;
            lab.registry.disconnect(&agent_id());

            assert!(agent.order().await.is_none());
            assert_eq!(lab.jobs.row(kept.id()).status(), Phase::Cancelled);
            assert!(lab.lines.0.lock().unwrap().is_empty());
        }

        #[tokio::test]
        async fn a_job_placed_on_a_stream_stays_through_the_hello_and_returns_when_it_ends() {
            let pipeline = PipelineBuilder::for_project_id(ProjectId::new("p")).build();
            let waiting = JobBuilder::new(&pipeline).build();
            let lab = lab(vec![waiting.clone()]).await;
            let mut agent = lab.open().await;
            let dispatcher = CallerContext::Service(ServiceIdentity::job_dispatcher());

            let placed = lab
                .actions
                .run(
                    &*lab.dispatch,
                    &dispatcher,
                    DispatchPendingJobs { agents: None },
                )
                .await
                .unwrap();
            assert!(matches!(
                agent.order().await,
                Some(agent_down::Payload::Dispatch(d))
                    if d.job_id.as_ref().is_some_and(|id| id.value == waiting.id().as_str())
            ));
            agent.hello(&[]).await;
            until(|| lab.touches() == 2).await;
            let quiet = tokio::time::timeout(Duration::from_millis(200), agent.orders.message());
            assert!(quiet.await.is_err(), "the hello sends no order");
            assert_eq!(placed.len(), 1);
            assert_eq!(lab.jobs.row(waiting.id()).agent_app_id(), Some(&agent_id()));

            lab.registry.disconnect(&agent_id());

            assert!(agent.order().await.is_none());
            until(|| lab.jobs.row(waiting.id()).agent_app_id().is_none()).await;
            assert_eq!(lab.jobs.row(waiting.id()).status(), Phase::Pending);
        }
    }
}
