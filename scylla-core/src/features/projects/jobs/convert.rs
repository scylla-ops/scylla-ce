//! The conversions of the jobs and of their logs to the proto messages, with the reason why a
//! pending job waits.

use scylla_domain::integrations::sources::ApprovalState;
use scylla_domain::projects::jobs::{
    needs_container, CheckoutRun, Job, JobApproval, JobStatus, LogBatch, LogStream, NodeRun,
    NodeStatus, PipelineFileRef,
};
use scylla_domain::projects::pipeline_files::FileError;
use scylla_domain::projects::pipelines::Node;
use scylla_proto::pipelines::v1 as pb;

use super::filter;
use crate::api::parse::{self, ToProto};
use crate::features::identity::agents::{Features, Registry};

/// Why a pending job with a checkout waits: no agent of its workspace that gets the code is
/// online.
const NO_AGENT_WITH_CHECKOUT: &str = "NO_AGENT_WITH_CHECKOUT";

/// Why a pending job with a container waits: no agent of its workspace with a container runtime,
/// and with the checkout when the job needs it, is online.
const NO_AGENT_WITH_CONTAINER: &str = "NO_AGENT_WITH_CONTAINER";

/// Returns the proto of a job, with the reason why it waits for an agent.
pub(super) fn job_of(job: Job, registry: &Registry) -> pb::Job {
    let reason = waiting_reason(&job, registry);
    let mut proto = job.to_proto();
    proto.waiting_reason = reason.unwrap_or_default().to_owned();
    proto
}

/// Returns why a pending job with no agent waits, when no online agent of its workspace has a
/// feature that it needs.
fn waiting_reason(job: &Job, registry: &Registry) -> Option<&'static str> {
    if job.status != JobStatus::Pending || job.agent_id.is_some() {
        return None;
    }
    let checkout = job.checkout.is_some();
    let containers = needs_container(&job.nodes);
    let only_checkout = Features {
        containers: false,
        checkout,
    };
    let both = Features {
        containers,
        checkout,
    };
    if checkout && !registry.can_serve(job.workspace_id, only_checkout) {
        Some(NO_AGENT_WITH_CHECKOUT)
    } else if containers && !registry.can_serve(job.workspace_id, both) {
        Some(NO_AGENT_WITH_CONTAINER)
    } else {
        None
    }
}

impl ToProto<pb::Job> for Job {
    fn to_proto(self) -> pb::Job {
        let changes = self.source.as_ref().map(|source| {
            let url = source.changes_url().unwrap_or_default();
            (source.changes_pipeline_files, url)
        });
        pb::Job {
            id: Some(self.id.to_proto()),
            pipeline_id: Some(self.pipeline_id.to_proto()),
            project_id: Some(self.project_id.to_proto()),
            workspace_id: Some(self.workspace_id.to_proto()),
            created_by: Some(self.created_by.to_proto()),
            rerun_of: self.rerun_of.map(ToProto::to_proto),
            status: status(self.status).into(),
            runs: self
                .runs
                .into_iter()
                .map(|one| run(one, &self.nodes))
                .collect(),
            nodes: self.nodes.into_iter().map(ToProto::to_proto).collect(),
            agent_id: self.agent_id.map(ToProto::to_proto),
            error: self.error.unwrap_or_default(),
            created_at: Some(parse::timestamp(self.created_at)),
            started_at: self.started_at.map(parse::timestamp),
            finished_at: self.finished_at.map(parse::timestamp),
            trigger_id: self.trigger_id.map(ToProto::to_proto),
            source: self.source.map(ToProto::to_proto),
            approval: self
                .approval
                .map(|one| approval(one, changes.unwrap_or_default())),
            pipeline_file: self.pipeline_file.map(file),
            file_errors: self.file_errors.into_iter().map(file_error).collect(),
            checkout: self.checkout.map(checkout),
            waiting_reason: String::new(),
            origin: filter::origin(self.origin).into(),
        }
    }
}

/// Returns the proto of the approval of a job, with the changes of its pull request.
fn approval(approval: JobApproval, (changes, url): (bool, String)) -> pb::JobApproval {
    let state = match approval.state {
        ApprovalState::Waiting => pb::ApprovalState::Waiting,
        ApprovalState::Approved => pb::ApprovalState::Approved,
        ApprovalState::Rejected => pb::ApprovalState::Rejected,
    };
    pb::JobApproval {
        state: state.into(),
        decided_by: approval.decided_by.map(ToProto::to_proto),
        decided_at: approval.decided_at.map(parse::timestamp),
        changes_pipeline_files: changes,
        changes_url: url,
    }
}

/// Returns the proto of a pipeline file.
fn file(file: PipelineFileRef) -> pb::PipelineFileRef {
    pb::PipelineFileRef {
        path: file.path,
        blob_sha: file.blob_sha,
        commit_sha: file.commit_sha,
        url: file.url,
    }
}

/// Returns the proto of an error of a pipeline file.
fn file_error(error: FileError) -> pb::FileError {
    pb::FileError {
        path: error.path,
        line: error.location.map_or(0, |at| at.line),
        column: error.location.map_or(0, |at| at.column),
        field: error.field,
        message: error.message,
        warning: false,
    }
}

/// Returns the proto of the checkout of a job.
fn checkout(checkout: CheckoutRun) -> pb::CheckoutRun {
    pb::CheckoutRun {
        status: node_status(checkout.status).into(),
        started_at: checkout.started_at.map(parse::timestamp),
        finished_at: checkout.finished_at.map(parse::timestamp),
        error: checkout.error.unwrap_or_default(),
    }
}

/// Returns the proto of the state of a job.
pub(crate) fn status(status: JobStatus) -> pb::JobStatus {
    match status {
        JobStatus::AwaitingApproval => pb::JobStatus::AwaitingApproval,
        JobStatus::Pending => pb::JobStatus::Pending,
        JobStatus::Running => pb::JobStatus::Running,
        JobStatus::Completed => pb::JobStatus::Completed,
        JobStatus::Failed => pb::JobStatus::Failed,
        JobStatus::Cancelled => pb::JobStatus::Cancelled,
        JobStatus::Orphaned => pb::JobStatus::Orphaned,
    }
}

/// Returns the proto of the state of a node.
fn node_status(status: NodeStatus) -> pb::NodeStatus {
    match status {
        NodeStatus::Pending => pb::NodeStatus::Pending,
        NodeStatus::Running => pb::NodeStatus::Running,
        NodeStatus::Completed => pb::NodeStatus::Completed,
        NodeStatus::Failed => pb::NodeStatus::Failed,
        NodeStatus::Skipped => pb::NodeStatus::Skipped,
        NodeStatus::Cancelled => pb::NodeStatus::Cancelled,
    }
}

/// Returns the proto of the state of a node in a job, with the most attempts of its node.
fn run(run: NodeRun, nodes: &[Node]) -> pb::NodeRun {
    let node = nodes.iter().find(|node| node.id == run.node);
    pb::NodeRun {
        max_attempts: node.map_or(1, |node| node.retries.saturating_add(1)),
        node_id: run.node.as_str().to_owned(),
        status: node_status(run.status).into(),
        attempt: run.attempt,
        started_at: run.started_at.map(parse::timestamp),
        finished_at: run.finished_at.map(parse::timestamp),
        exit_code: run.exit_code,
        error: run.error.unwrap_or_default(),
    }
}

impl ToProto<pb::TailJobLogsResponse> for LogBatch {
    fn to_proto(self) -> pb::TailJobLogsResponse {
        let lines = self
            .lines
            .into_iter()
            .map(|line| {
                let stream = match line.stream {
                    LogStream::Stdout => pb::LogStream::Stdout,
                    LogStream::Stderr => pb::LogStream::Stderr,
                };
                pb::LogLine {
                    stream: stream.into(),
                    text: line.text,
                    at: Some(parse::timestamp(line.at)),
                }
            })
            .collect();
        pb::TailJobLogsResponse {
            cursor: self.id.to_string(),
            node_id: self.node,
            lines,
        }
    }
}
