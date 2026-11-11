//! The conversions of the jobs and of their logs to the proto messages.

use scylla_domain::projects::jobs::{Job, JobStatus, LogBatch, LogStream, NodeRun, NodeStatus};
use scylla_proto::pipelines::v1 as pb;

use crate::api::parse::{self, ToProto};

impl ToProto<pb::Job> for Job {
    fn to_proto(self) -> pb::Job {
        pb::Job {
            id: Some(self.id.to_proto()),
            pipeline_id: Some(self.pipeline_id.to_proto()),
            project_id: Some(self.project_id.to_proto()),
            workspace_id: Some(self.workspace_id.to_proto()),
            created_by: Some(self.created_by.to_proto()),
            rerun_of: self.rerun_of.map(ToProto::to_proto),
            status: status(self.status).into(),
            nodes: self.nodes.into_iter().map(ToProto::to_proto).collect(),
            runs: self.runs.into_iter().map(run).collect(),
            agent_id: self.agent_id.map(ToProto::to_proto),
            error: self.error.unwrap_or_default(),
            created_at: Some(parse::timestamp(self.created_at)),
            started_at: self.started_at.map(parse::timestamp),
            finished_at: self.finished_at.map(parse::timestamp),
            trigger_id: self.trigger_id.map(ToProto::to_proto),
            source: None,
            approval: None,
            pipeline_file: None,
            file_errors: Vec::new(),
            checkout: None,
            waiting_reason: String::new(),
        }
    }
}

/// Returns the proto of the state of a job.
fn status(status: JobStatus) -> pb::JobStatus {
    match status {
        JobStatus::Pending => pb::JobStatus::Pending,
        JobStatus::Running => pb::JobStatus::Running,
        JobStatus::Completed => pb::JobStatus::Completed,
        JobStatus::Failed => pb::JobStatus::Failed,
        JobStatus::Cancelled => pb::JobStatus::Cancelled,
        JobStatus::Orphaned => pb::JobStatus::Orphaned,
    }
}

/// Returns the proto of the state of a node.
fn run(run: NodeRun) -> pb::NodeRun {
    let status = match run.status {
        NodeStatus::Pending => pb::NodeStatus::Pending,
        NodeStatus::Running => pb::NodeStatus::Running,
        NodeStatus::Completed => pb::NodeStatus::Completed,
        NodeStatus::Failed => pb::NodeStatus::Failed,
        NodeStatus::Skipped => pb::NodeStatus::Skipped,
        NodeStatus::Cancelled => pb::NodeStatus::Cancelled,
    };
    pb::NodeRun {
        node_id: run.node.as_str().to_owned(),
        status: status.into(),
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
