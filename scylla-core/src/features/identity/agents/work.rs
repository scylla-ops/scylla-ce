//! The jobs on the stream of an agent: the placement of the pending jobs, the events and the
//! logs of the agent, and the messages of the server.

use scylla_domain::authz::Object;
use scylla_domain::projects::jobs::{JobEvent, JobId, LogLine, LogStream};
use scylla_domain::projects::pipelines::{NodeId, PipelineHolder};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{AccessError, Gate, Principal};
use scylla_proto::agents::v1 as pb;
use scylla_proto::agents::v1::open_stream_response::Message as Down;
use scylla_proto::pipelines::v1 as pipelines_pb;
use time::OffsetDateTime;
use tonic::Status;

use super::stream::{Outbox, Stream};
use crate::api::parse::{self, ToProto};
use crate::features::projects::jobs::runs::{self, Assignment, Reply};
use crate::features::projects::pipelines;

/// Sends one message to the agent. Returns `false` when the agent is gone.
pub(super) async fn send(out: &Outbox, message: Down) -> bool {
    let message = pb::OpenStreamResponse {
        message: Some(message),
    };
    out.send(Ok(message)).await.is_ok()
}

/// Makes the answer to the hello.
pub(super) fn welcome(expires_at: OffsetDateTime) -> Down {
    Down::Welcome(pb::Welcome {
        expires_at: Some(parse::timestamp(expires_at)),
    })
}

/// Makes the answer to a renewal.
pub(super) fn renewed(expires_at: OffsetDateTime) -> Down {
    Down::Renewed(pb::Renewed {
        expires_at: Some(parse::timestamp(expires_at)),
    })
}

/// Makes the command that stops a job.
pub(super) fn cancel(job: JobId) -> Down {
    Down::Cancel(pb::Cancel {
        job_id: Some(job.to_proto()),
    })
}

/// Returns `true` if the agent has the role `runner` in its workspace. It is not a route check.
pub(super) async fn may_run(
    gate: &dyn Gate,
    principal: &Principal,
    workspace: WorkspaceId,
) -> Result<bool, AccessError> {
    let name = PipelineHolder::RUN_JOBS.name();
    let answers = gate
        .permissions(principal, &Object::of(workspace), Some(workspace), &[name])
        .await?;
    Ok(answers.first().copied().unwrap_or(false))
}

/// Checks the jobs of the agent after its hello, and returns the jobs that it must stop.
pub(super) async fn reconcile(stream: &Stream, running: &[JobId]) -> Result<Vec<JobId>, Status> {
    let (stop, released) = runs::reconcile(&stream.pool, stream.app, running).await?;
    if released {
        stream.registry.wake_all();
    }
    Ok(stop)
}

/// Gives pending jobs to an agent with the role, while it holds fewer jobs than its capacity.
/// Returns `false` when the agent is gone.
pub(super) async fn fill(stream: &Stream, out: &Outbox) -> Result<bool, Status> {
    let Some(workspace) = stream.workspace.filter(|_| stream.runner) else {
        return Ok(true);
    };
    let sealer = stream.sealer.as_deref();
    while runs::held(&stream.pool, stream.app).await? < stream.capacity {
        let claimed = runs::claim(
            &stream.pool,
            sealer,
            workspace,
            stream.app,
            stream.containers,
        );
        let Some(assignment) = claimed.await? else {
            break;
        };
        tracing::info!(event = "job.dispatched", job = %assignment.job.id, app = %stream.app);
        if !send(out, dispatch(assignment)).await {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Applies an event of the agent. A job that the agent must not run gets a cancel, and the end
/// of a job gives room for another job.
pub(super) async fn event(
    stream: &Stream,
    event: pb::JobEvent,
    out: &Outbox,
) -> Result<bool, Status> {
    let job: JobId = parse::required(event.job_id.clone(), "event.job_id")?;
    let domain = event_of(event)?;
    match runs::apply(&stream.pool, stream.app, job, &domain).await? {
        Reply::Nothing => Ok(true),
        Reply::Cancel => Ok(send(out, cancel(job)).await),
        Reply::Ended => {
            stream.registry.wake_all();
            fill(stream, out).await
        }
    }
}

/// Stores a batch of lines. A job that the agent must not run gets a cancel.
pub(super) async fn logs(
    stream: &Stream,
    batch: pb::LogBatch,
    out: &Outbox,
) -> Result<bool, Status> {
    // Read the batch: at most 1000 lines, each cut at 64 KiB.
    let job: JobId = parse::required(batch.job_id, "logs.job_id")?;
    if batch.lines.is_empty() || batch.lines.len() > LogLine::MAX_BATCH {
        return Err(Status::invalid_argument(
            "logs.lines must have 1 to 1000 lines",
        ));
    }
    let now = OffsetDateTime::now_utc();
    let lines: Vec<LogLine> = batch
        .lines
        .into_iter()
        .map(|line| LogLine {
            stream: match line.stream() {
                pipelines_pb::LogStream::Stderr => LogStream::Stderr,
                _ => LogStream::Stdout,
            },
            text: LogLine::cut(line.text),
            at: line
                .at
                .and_then(|at| {
                    let nanos = i128::from(at.seconds) * 1_000_000_000 + i128::from(at.nanos);
                    OffsetDateTime::from_unix_timestamp_nanos(nanos).ok()
                })
                .unwrap_or(now),
        })
        .collect();

    // Store it.
    match runs::append(&stream.pool, stream.app, job, &batch.node_id, &lines).await? {
        Reply::Cancel => Ok(send(out, cancel(job)).await),
        Reply::Nothing | Reply::Ended => Ok(true),
    }
}

/// Reads an event of the agent.
fn event_of(event: pb::JobEvent) -> Result<JobEvent, Status> {
    let node = || {
        NodeId::parse(event.node_id.clone())
            .map_err(|error| Status::invalid_argument(format!("event.node_id {error}")))
    };
    Ok(match event.kind() {
        pb::EventKind::JobStarted => JobEvent::JobStarted,
        pb::EventKind::NodeStarted => JobEvent::NodeStarted(node()?),
        pb::EventKind::NodeRetrying => JobEvent::NodeRetrying {
            node: node()?,
            attempt: event.attempt,
        },
        pb::EventKind::NodeCompleted => JobEvent::NodeCompleted {
            node: node()?,
            exit_code: event.exit_code,
        },
        pb::EventKind::NodeFailed => JobEvent::NodeFailed {
            node: node()?,
            exit_code: event.exit_code,
            error: event.error.clone(),
        },
        pb::EventKind::NodeSkipped => JobEvent::NodeSkipped(node()?),
        pb::EventKind::JobCompleted => JobEvent::JobCompleted,
        pb::EventKind::JobFailed => JobEvent::JobFailed(event.error.clone()),
        pb::EventKind::Unspecified => {
            return Err(Status::invalid_argument("event.kind is required"));
        }
    })
}

/// Makes the dispatch of a job, with the values of its secrets.
fn dispatch(assignment: Assignment) -> Down {
    let Assignment { job, env } = assignment;
    let nodes = job
        .nodes
        .into_iter()
        .zip(env)
        .map(|(node, env)| pb::DispatchNode {
            id: node.id.as_str().to_owned(),
            needs: node
                .needs
                .iter()
                .map(|need| need.as_str().to_owned())
                .collect(),
            step: Some(match node.step {
                scylla_domain::projects::pipelines::Step::Exec(argv) => {
                    pb::dispatch_node::Step::Exec(pipelines_pb::Exec { argv })
                }
                scylla_domain::projects::pipelines::Step::Script { shell, source } => {
                    pb::dispatch_node::Step::Script(pipelines::script(shell, source))
                }
            }),
            env: env
                .into_iter()
                .map(|var| pb::DispatchEnv {
                    name: var.name,
                    value: var.value,
                    masked: var.masked,
                })
                .collect(),
            timeout_seconds: node.timeout_seconds,
            retries: node.retries,
            working_dir: node.working_dir.unwrap_or_default(),
            container: pipelines::container(node.runtime),
        })
        .collect();
    Down::Dispatch(pb::Dispatch {
        job_id: Some(job.id.to_proto()),
        pipeline_id: Some(job.pipeline_id.to_proto()),
        project_id: Some(job.project_id.to_proto()),
        nodes,
    })
}
