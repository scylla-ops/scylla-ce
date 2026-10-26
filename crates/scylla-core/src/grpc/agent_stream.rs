//! The frames of the agent stream: the work that one chunk of frames gives, the orders that go
//! down the stream, and the conversions of the agent messages.

use crate::application::JobDispatch;
use crate::application::agent::AgentOrder;
use crate::domain::agent::AgentHost;
use crate::domain::clock;
use crate::domain::ids::JobId;
use crate::domain::job::JobLog;
use crate::domain::pipeline::{NodeId, Step};
use crate::grpc::convert::dt;
use crate::infrastructure::messaging::agent_registry::ConnHandle;
use futures_util::{StreamExt, future, stream};
use scylla_proto::agent::v1::{
    AgentDown, AgentHello, AgentNode, AgentUp, CancelJob, JobDispatch as ProtoJobDispatch,
    JobLogLine, JobStatus, ResolvedEnv, agent_down, agent_node, agent_up,
};
use scylla_proto::common::v1 as common;
use scylla_proto::exec::v1 as exec;
use std::sync::atomic::Ordering;
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Status;
use tracing::warn;

/// The frames the reader takes at once: the lines among them go to the store in one batch.
pub const READ_CHUNK: usize = 512;

#[derive(Debug)]
pub enum Work {
    Logs(JobId, Vec<JobLog>),
    Status(JobStatus),
    Hello(AgentHello),
    Undecodable(Status),
}

/// The frames of one chunk as work, in order. The lines that come between two other frames
/// form one batch for each job, so a status never overtakes a line sent before it.
pub fn work(chunk: Vec<Result<AgentUp, Status>>) -> Vec<Work> {
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
pub fn down_stream(conn: ConnHandle) -> impl Stream<Item = Result<AgentDown, Status>> + Send {
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

pub fn hello_to_domain(hello: &AgentHello) -> AgentHost {
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

pub fn log_line_to_domain(line: &JobLogLine) -> Option<JobLog> {
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
    use crate::infrastructure::InMemoryAgentRegistry;
    use scylla_proto::agent::v1::job_status::{Event, JobStarted};
    use std::sync::Arc;
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
}
