//! Wire to the inputs of a use case, its outcome to wire. The handler holds none of it.

use crate::application::pagination::PaginationParams;
use crate::grpc::convert::{Parse, id, optional_id, ts, wrap};
use crate::grpc::mappers::proto_to_domain_pagination;
use scylla_domain::domain::ids::{AppId, JobId, OrganizationId, PipelineId, ProjectId};
use scylla_domain::domain::job::JobOrigin;
use scylla_domain::domain::job::{
    Job, JobNode, JobState, NodeExecution, NodeOutcome, TerminalOutcome,
};
use scylla_proto::job::v1::{
    CancelJobRequest, DeleteJobRequest, GetJobRequest, Job as ProtoJob, JobNode as ProtoJobNode,
    JobOutcome, ListJobsRequest, ListJobsResponse, ListOrganizationJobsRequest,
    ListOrganizationJobsResponse, ListPipelineJobsRequest, ListPipelineJobsResponse,
    ListProjectJobsRequest, ListProjectJobsResponse, NodeOutcome as ProtoNodeOutcome, job,
    job_node,
};
use tonic::Status;

pub fn job_to_proto(job: &Job) -> ProtoJob {
    ProtoJob {
        job_id: wrap(job.id().to_string()),
        pipeline_id: wrap(job.pipeline_id().to_string()),
        node_executions: job
            .node_executions()
            .iter()
            .map(job_node_to_proto)
            .collect(),
        created_at: ts(job.created_at()),
        updated_at: ts(job.updated_at()),
        state: Some(job_state_to_proto(job)),
        origin: Some(origin_to_proto(job.origin())),
    }
}

fn job_state_to_proto(job: &Job) -> job::State {
    match job.state() {
        JobState::Pending => job::State::Pending(job::Pending {
            agent_id: job.agent_app_id().and_then(|id| wrap(id.to_string())),
        }),
        JobState::Running { started_at } => job::State::Running(job::Running {
            started_at: ts(*started_at),
        }),
        JobState::Terminal {
            outcome,
            started_at,
            finished_at,
        } => job::State::Terminal(job::Terminal {
            outcome: job_outcome_to_proto(*outcome) as i32,
            started_at: started_at.and_then(ts),
            finished_at: ts(*finished_at),
        }),
    }
}

fn job_outcome_to_proto(outcome: TerminalOutcome) -> JobOutcome {
    match outcome {
        TerminalOutcome::Completed => JobOutcome::Completed,
        TerminalOutcome::Failed => JobOutcome::Failed,
        TerminalOutcome::Cancelled => JobOutcome::Cancelled,
        TerminalOutcome::Orphaned => JobOutcome::Orphaned,
    }
}

fn origin_to_proto(origin: &JobOrigin) -> job::Origin {
    match origin {
        JobOrigin::Human { user_id } => job::Origin::Human(job::Human {
            user_id: wrap(user_id.to_string()),
        }),
        JobOrigin::App { app_id } => job::Origin::App(job::App {
            app_id: wrap(app_id.to_string()),
        }),
        JobOrigin::Cron { trigger_id } => job::Origin::Cron(job::Cron {
            trigger_id: wrap(trigger_id.to_string()),
        }),
        JobOrigin::Webhook {
            trigger_id,
            delivery_id,
        } => job::Origin::Webhook(job::Webhook {
            trigger_id: wrap(trigger_id.to_string()),
            delivery_id: delivery_id.clone(),
        }),
    }
}

pub fn job_node_to_proto(node: &JobNode) -> ProtoJobNode {
    ProtoJobNode {
        node_id: wrap(node.node_id().to_string()),
        execution: Some(node_execution_to_proto(node.execution())),
    }
}

fn node_execution_to_proto(execution: &NodeExecution) -> job_node::Execution {
    match execution {
        NodeExecution::Pending => job_node::Execution::Pending(job_node::Pending {}),
        NodeExecution::Running { started_at } => job_node::Execution::Running(job_node::Running {
            started_at: ts(*started_at),
        }),
        NodeExecution::Finished {
            started_at,
            finished_at,
            outcome,
        } => job_node::Execution::Finished(job_node::Finished {
            outcome: node_outcome_to_proto(*outcome) as i32,
            started_at: started_at.and_then(ts),
            finished_at: ts(*finished_at),
        }),
    }
}

fn node_outcome_to_proto(outcome: NodeOutcome) -> ProtoNodeOutcome {
    match outcome {
        NodeOutcome::Completed => ProtoNodeOutcome::Completed,
        NodeOutcome::Failed => ProtoNodeOutcome::Failed,
        NodeOutcome::Cancelled => ProtoNodeOutcome::Cancelled,
        NodeOutcome::Skipped => ProtoNodeOutcome::Skipped,
    }
}

impl Parse for GetJobRequest {
    type Into = JobId;

    fn parse(self) -> Result<JobId, Status> {
        id(self.job_id, "job_id")
    }
}

impl Parse for CancelJobRequest {
    type Into = JobId;

    fn parse(self) -> Result<JobId, Status> {
        id(self.job_id, "job_id")
    }
}

impl Parse for DeleteJobRequest {
    type Into = JobId;

    fn parse(self) -> Result<JobId, Status> {
        id(self.job_id, "job_id")
    }
}

impl Parse for ListJobsRequest {
    type Into = Option<PaginationParams>;

    fn parse(self) -> Result<Option<PaginationParams>, Status> {
        Ok(proto_to_domain_pagination(self.pagination))
    }
}

impl Parse for ListPipelineJobsRequest {
    type Into = (PipelineId, Option<PaginationParams>);

    fn parse(self) -> Result<Self::Into, Status> {
        let pipeline_id = id(self.pipeline_id, "pipeline_id")?;
        Ok((pipeline_id, proto_to_domain_pagination(self.pagination)))
    }
}

impl Parse for ListProjectJobsRequest {
    type Into = (ProjectId, Option<PaginationParams>);

    fn parse(self) -> Result<Self::Into, Status> {
        let project_id = id(self.project_id, "project_id")?;
        Ok((project_id, proto_to_domain_pagination(self.pagination)))
    }
}

impl Parse for ListOrganizationJobsRequest {
    type Into = (OrganizationId, Option<AppId>, Option<PaginationParams>);

    fn parse(self) -> Result<Self::Into, Status> {
        let organization_id = id(self.organization_id, "organization_id")?;
        let agent_id = optional_id(self.agent_id, "agent_id")?;
        Ok((
            organization_id,
            agent_id,
            proto_to_domain_pagination(self.pagination),
        ))
    }
}

page_response!(
    Job => jobs: job_to_proto;
    ListJobsResponse,
    ListPipelineJobsResponse,
    ListProjectJobsResponse,
    ListOrganizationJobsResponse,
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc::convert::Parse;
    use scylla_proto::common::v1 as common;
    use tonic::Code;

    #[test]
    fn a_get_request_becomes_the_job_id() {
        let id = GetJobRequest {
            job_id: wrap("job-1"),
        }
        .parse()
        .unwrap();

        assert_eq!(id.as_str(), "job-1");
    }

    #[test]
    fn a_missing_id_is_an_invalid_argument() {
        let err = DeleteJobRequest {
            job_id: None::<common::JobId>,
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "missing job_id");
    }

    #[test]
    fn a_pending_job_names_the_agent_it_is_placed_on() {
        use crate::domain::ids::{AppId, ProjectId};
        use crate::test_support::jobs::JobBuilder;
        use crate::test_support::pipelines::PipelineBuilder;
        let pipeline = PipelineBuilder::for_project_id(ProjectId::new("p")).build();
        let agent = |job: &Job| match job_to_proto(job).state {
            Some(job::State::Pending(pending)) => pending.agent_id.map(|id| id.value),
            other => panic!("expected a pending job, got {other:?}"),
        };

        let placed = JobBuilder::new(&pipeline)
            .agent(AppId::new("agent-1"))
            .build();
        let waiting = JobBuilder::new(&pipeline).build();

        assert_eq!(agent(&placed).as_deref(), Some("agent-1"));
        assert_eq!(agent(&waiting), None);
    }

    #[test]
    fn a_cancel_request_becomes_the_job_id() {
        let id = CancelJobRequest {
            job_id: wrap("job-1"),
        }
        .parse()
        .unwrap();

        assert_eq!(id.as_str(), "job-1");
    }

    #[test]
    fn a_pipeline_list_request_carries_its_pipeline_and_page() {
        let (pipeline_id, pagination) = ListPipelineJobsRequest {
            pipeline_id: wrap("pipe-1"),
            pagination: None,
        }
        .parse()
        .unwrap();

        assert_eq!(pipeline_id.as_str(), "pipe-1");
        assert!(pagination.is_none());
    }

    #[test]
    fn an_organization_list_request_carries_its_agent() {
        let (organization_id, agent_id, _) = ListOrganizationJobsRequest {
            organization_id: wrap("acme"),
            pagination: None,
            agent_id: wrap("agent-1"),
        }
        .parse()
        .unwrap();
        let (_, unfiltered, _) = ListOrganizationJobsRequest {
            organization_id: wrap("acme"),
            pagination: None,
            agent_id: None,
        }
        .parse()
        .unwrap();

        assert_eq!(organization_id.as_str(), "acme");
        assert_eq!(agent_id.unwrap().as_str(), "agent-1");
        assert!(unfiltered.is_none());
    }

    #[test]
    fn an_empty_agent_id_is_an_invalid_argument() {
        let err = ListOrganizationJobsRequest {
            organization_id: wrap("acme"),
            pagination: None,
            agent_id: wrap(""),
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "invalid agent_id");
    }
}
