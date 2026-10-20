//! Wire to query, query outcome to wire. The handler holds none of it.

use crate::application::job::{ListJobLogs, TailJobLogs};
use crate::grpc::convert::{Parse, id, optional, ts, wrap};
use crate::grpc::mappers::proto_to_domain_pagination;
use scylla_domain::domain::job::JobLog;
use scylla_domain::domain::pipeline::NodeId;
use scylla_proto::common::v1 as common;
use scylla_proto::convert::log_stream_to_proto;
use scylla_proto::job::v1::{
    JobLogEntry, ListJobLogsRequest, ListJobLogsResponse, TailJobLogsRequest,
};
use tonic::Status;

pub fn job_log_to_proto(log: &JobLog) -> JobLogEntry {
    JobLogEntry {
        job_log_id: wrap(log.id().to_string()),
        job_id: wrap(log.job_id().to_string()),
        node_id: wrap(log.node_id().to_string()),
        stream: log_stream_to_proto(*log.stream()) as i32,
        line: log.line().to_string(),
        timestamp: ts(log.timestamp()),
    }
}

fn node_filter(field: Option<common::NodeId>) -> Result<Option<NodeId>, Status> {
    optional(field)
        .map(NodeId::new)
        .transpose()
        .map_err(|e| Status::invalid_argument(format!("Invalid node_id: {e}")))
}

impl Parse for ListJobLogsRequest {
    type Into = ListJobLogs;

    fn parse(self) -> Result<ListJobLogs, Status> {
        Ok(ListJobLogs {
            job_id: id(self.job_id, "job_id")?,
            node_id: node_filter(self.node_id)?,
            pagination: proto_to_domain_pagination(self.pagination),
        })
    }
}

impl Parse for TailJobLogsRequest {
    type Into = TailJobLogs;

    fn parse(self) -> Result<TailJobLogs, Status> {
        Ok(TailJobLogs {
            job_id: id(self.job_id, "job_id")?,
            node_id: node_filter(self.node_id)?,
        })
    }
}

page_response!(JobLog => logs: job_log_to_proto; ListJobLogsResponse);

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    #[test]
    fn a_list_request_carries_its_job_node_and_page() {
        let query = ListJobLogsRequest {
            job_id: wrap("job-1"),
            node_id: wrap("build"),
            pagination: None,
        }
        .parse()
        .unwrap();

        assert_eq!(query.job_id.as_str(), "job-1");
        assert_eq!(query.node_id.unwrap().as_str(), "build");
        assert!(query.pagination.is_none());
    }

    #[test]
    fn an_unset_node_reads_every_node() {
        let query = TailJobLogsRequest {
            job_id: wrap("job-1"),
            node_id: None,
        }
        .parse()
        .unwrap();

        assert!(query.node_id.is_none());
    }

    #[test]
    fn an_invalid_node_is_an_invalid_argument() {
        let err = TailJobLogsRequest {
            job_id: wrap("job-1"),
            node_id: wrap("not a node"),
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert!(err.message().starts_with("Invalid node_id: "));
    }

    #[test]
    fn a_missing_job_is_an_invalid_argument() {
        let err = ListJobLogsRequest {
            job_id: None,
            node_id: None,
            pagination: None,
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "missing job_id");
    }
}
