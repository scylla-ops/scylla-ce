//! Wire to the inputs of a use case, its outcome to wire. The handler holds none of it.

use crate::application::pagination::PaginationParams;
use crate::grpc::convert::{Parse, id, required, ts, valid, wrap};
use crate::grpc::mappers::{domain_error_to_status, proto_to_domain_pagination};
use scylla_domain::domain::ids::{OrganizationId, PipelineId, ProjectId};
use scylla_domain::domain::pipeline::{
    EnvKey, EnvSource, EnvVar as DomainEnvVar, NodeId, PipelineName, Step, WorkingDir,
};
use scylla_domain::domain::pipeline::{Pipeline, PipelineNode as DomainPipelineNode};
use scylla_domain::domain::secret::SecretName;
use scylla_proto::common::v1 as common;
use scylla_proto::exec::v1 as exec;
use scylla_proto::pipeline::v1::{
    CreatePipelineRequest, DeletePipelineRequest, EnvVar, GetPipelineRequest,
    ListOrganizationPipelinesRequest, ListOrganizationPipelinesResponse, ListPipelinesRequest,
    ListPipelinesResponse, ListProjectPipelinesRequest, ListProjectPipelinesResponse,
    Pipeline as ProtoPipeline, PipelineNode, PipelineSummary, RunPipelineRequest,
    UpdatePipelineRequest, env_var, pipeline_node,
};
use tonic::Status;

pub fn pipeline_to_proto(pipeline: &Pipeline) -> ProtoPipeline {
    ProtoPipeline {
        pipeline_id: wrap(pipeline.id().to_string()),
        project_id: wrap(pipeline.project_id().to_string()),
        name: pipeline.name().to_string(),
        nodes: pipeline
            .nodes()
            .iter()
            .map(pipeline_node_to_proto)
            .collect(),
        created_at: ts(pipeline.created_at()),
        updated_at: ts(pipeline.updated_at()),
    }
}

pub fn pipeline_to_proto_summary(pipeline: &Pipeline) -> PipelineSummary {
    PipelineSummary {
        pipeline_id: wrap(pipeline.id().to_string()),
        project_id: wrap(pipeline.project_id().to_string()),
        name: pipeline.name().to_string(),
        node_count: u32::try_from(pipeline.nodes().len()).unwrap_or(u32::MAX),
        created_at: ts(pipeline.created_at()),
        updated_at: ts(pipeline.updated_at()),
    }
}

pub fn pipeline_node_to_proto(node: &DomainPipelineNode) -> PipelineNode {
    PipelineNode {
        node_id: wrap(node.id().to_string()),
        deps: node
            .deps()
            .iter()
            .map(|d: &NodeId| common::NodeId {
                value: d.to_string(),
            })
            .collect(),
        working_dir: node
            .working_dir()
            .map(|wd| wd.as_str().to_string())
            .unwrap_or_default(),
        env: node.env().iter().map(env_to_proto).collect(),
        step: Some(step_to_proto(node.step())),
    }
}

fn env_to_proto(ev: &DomainEnvVar) -> EnvVar {
    let source = match ev.source() {
        EnvSource::Literal(v) => env_var::Source::Value(v.to_string()),
        EnvSource::Secret(name) => env_var::Source::SecretRef(name.as_str().to_string()),
    };
    EnvVar {
        key: ev.key().to_string(),
        source: Some(source),
    }
}

fn step_to_proto(step: &Step) -> pipeline_node::Step {
    match step {
        Step::Exec { command, args } => pipeline_node::Step::Exec(exec::ExecStep {
            command: command.to_string(),
            args: args.iter().map(ToString::to_string).collect(),
        }),
        Step::Script { script, shell } => pipeline_node::Step::Script(exec::ScriptStep {
            script: script.to_string(),
            shell: scylla_proto::convert::shell_to_proto(*shell) as i32,
        }),
    }
}

fn within(context: String) -> impl FnOnce(Status) -> Status {
    move |s| Status::new(s.code(), format!("{context}: {}", s.message()))
}

fn proto_node_to_domain(n: PipelineNode) -> Result<DomainPipelineNode, Status> {
    let node_id = valid(required(n.node_id, "node_id")?, NodeId::new)?;
    let context = format!("node '{node_id}'");
    let node = move || {
        let deps = n
            .deps
            .into_iter()
            .map(|d| valid(d.value, NodeId::new))
            .collect::<Result<_, _>>()?;
        let working_dir = match n.working_dir.trim() {
            "" => None,
            s => Some(valid(s, WorkingDir::new)?),
        };
        let env = n
            .env
            .into_iter()
            .map(|e| {
                let key = e.key.clone();
                proto_env_to_domain(e).map_err(within(format!("env '{key}'")))
            })
            .collect::<Result<_, _>>()?;
        let step = match n.step {
            Some(pipeline_node::Step::Exec(e)) => {
                Step::exec(e.command, e.args).map_err(domain_error_to_status)?
            }
            Some(pipeline_node::Step::Script(s)) => {
                Step::script(s.script, scylla_proto::convert::shell_from_proto(s.shell))
                    .map_err(domain_error_to_status)?
            }
            None => {
                return Err(Status::invalid_argument("missing step (exec or script)"));
            }
        };
        Ok(DomainPipelineNode::new(
            node_id,
            deps,
            step,
            working_dir,
            env,
        ))
    };
    node().map_err(within(context))
}

fn proto_env_to_domain(e: EnvVar) -> Result<DomainEnvVar, Status> {
    let key = valid(e.key, EnvKey::new)?;
    match e.source {
        Some(env_var::Source::Value(v)) => valid(v, |v| DomainEnvVar::literal(key, v)),
        Some(env_var::Source::SecretRef(name)) => {
            Ok(DomainEnvVar::secret(key, valid(name, SecretName::new)?))
        }
        None => Err(Status::invalid_argument("missing value or secret_ref")),
    }
}

fn proto_nodes_to_domain(nodes: Vec<PipelineNode>) -> Result<Vec<DomainPipelineNode>, Status> {
    nodes.into_iter().map(proto_node_to_domain).collect()
}

impl Parse for CreatePipelineRequest {
    type Into = (ProjectId, PipelineName, Vec<DomainPipelineNode>);

    fn parse(self) -> Result<Self::Into, Status> {
        let name = valid(self.name, PipelineName::new)?;
        let project_id = id(self.project_id, "project_id")?;
        let nodes = proto_nodes_to_domain(self.nodes)?;
        Ok((project_id, name, nodes))
    }
}

impl Parse for GetPipelineRequest {
    type Into = PipelineId;

    fn parse(self) -> Result<PipelineId, Status> {
        id(self.pipeline_id, "pipeline_id")
    }
}

impl Parse for UpdatePipelineRequest {
    type Into = (
        PipelineId,
        Option<PipelineName>,
        Option<Vec<DomainPipelineNode>>,
    );

    // An empty node list leaves the nodes as they are: the wire has no way to send "no change".
    fn parse(self) -> Result<Self::Into, Status> {
        let pipeline_id = id(self.pipeline_id, "pipeline_id")?;
        let name = self.name.map(|n| valid(n, PipelineName::new)).transpose()?;
        let nodes = if self.nodes.is_empty() {
            None
        } else {
            Some(proto_nodes_to_domain(self.nodes)?)
        };
        Ok((pipeline_id, name, nodes))
    }
}

impl Parse for DeletePipelineRequest {
    type Into = PipelineId;

    fn parse(self) -> Result<PipelineId, Status> {
        id(self.pipeline_id, "pipeline_id")
    }
}

impl Parse for RunPipelineRequest {
    type Into = PipelineId;

    fn parse(self) -> Result<PipelineId, Status> {
        id(self.pipeline_id, "pipeline_id")
    }
}

impl Parse for ListPipelinesRequest {
    type Into = Option<PaginationParams>;

    fn parse(self) -> Result<Option<PaginationParams>, Status> {
        Ok(proto_to_domain_pagination(self.pagination))
    }
}

impl Parse for ListProjectPipelinesRequest {
    type Into = (ProjectId, Option<PaginationParams>);

    fn parse(self) -> Result<Self::Into, Status> {
        let project_id = id(self.project_id, "project_id")?;
        Ok((project_id, proto_to_domain_pagination(self.pagination)))
    }
}

impl Parse for ListOrganizationPipelinesRequest {
    type Into = (OrganizationId, Option<PaginationParams>);

    fn parse(self) -> Result<Self::Into, Status> {
        let organization_id = id(self.organization_id, "organization_id")?;
        Ok((organization_id, proto_to_domain_pagination(self.pagination)))
    }
}

page_response!(
    Pipeline => pipelines: pipeline_to_proto_summary;
    ListPipelinesResponse,
    ListProjectPipelinesResponse,
    ListOrganizationPipelinesResponse,
);

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    fn exec_node(id: &str) -> PipelineNode {
        PipelineNode {
            node_id: wrap(id),
            deps: Vec::new(),
            working_dir: String::new(),
            env: vec![EnvVar {
                key: "TOKEN".into(),
                source: Some(env_var::Source::SecretRef("API_TOKEN".into())),
            }],
            step: Some(pipeline_node::Step::Exec(exec::ExecStep {
                command: "echo".into(),
                args: vec!["hi".into()],
            })),
        }
    }

    #[test]
    fn a_create_request_becomes_the_validated_inputs() {
        let (project_id, name, nodes) = CreatePipelineRequest {
            project_id: wrap("proj-1"),
            name: " build ".into(),
            nodes: vec![exec_node("a")],
        }
        .parse()
        .unwrap();

        assert_eq!(project_id.as_str(), "proj-1");
        assert_eq!(name.as_str(), "build");
        assert_eq!(nodes.len(), 1);
        assert!(matches!(
            nodes[0].env()[0].source(),
            EnvSource::Secret(name) if name.as_str() == "API_TOKEN"
        ));
    }

    #[test]
    fn a_missing_project_id_is_an_invalid_argument() {
        let err = CreatePipelineRequest {
            project_id: None::<common::ProjectId>,
            name: "build".into(),
            nodes: Vec::new(),
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "missing project_id");
    }

    #[test]
    fn a_node_without_a_step_is_an_invalid_argument() {
        let err = CreatePipelineRequest {
            project_id: wrap("proj-1"),
            name: "build".into(),
            nodes: vec![PipelineNode {
                step: None,
                ..exec_node("a")
            }],
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "node 'a': missing step (exec or script)");
    }

    #[test]
    fn an_env_var_without_a_source_is_an_invalid_argument() {
        let mut node = exec_node("a");
        node.env[0].source = None;
        let err = CreatePipelineRequest {
            project_id: wrap("proj-1"),
            name: "build".into(),
            nodes: vec![node],
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(
            err.message(),
            "node 'a': env 'TOKEN': missing value or secret_ref"
        );
    }

    #[test]
    fn a_bad_secret_ref_names_its_node_and_env_key() {
        let mut node = exec_node("n1");
        node.env[0] = EnvVar {
            key: "X".into(),
            source: Some(env_var::Source::SecretRef("bad name".into())),
        };
        let err = CreatePipelineRequest {
            project_id: wrap("proj-1"),
            name: "build".into(),
            nodes: vec![node],
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert!(
            err.message().starts_with("node 'n1': env 'X': "),
            "{}",
            err.message()
        );
    }

    #[test]
    fn an_update_with_no_nodes_leaves_them_unchanged() {
        let (id, name, nodes) = UpdatePipelineRequest {
            pipeline_id: wrap("pl-1"),
            name: None,
            nodes: Vec::new(),
        }
        .parse()
        .unwrap();

        assert_eq!(id.as_str(), "pl-1");
        assert!(name.is_none());
        assert!(nodes.is_none());
    }

    #[test]
    fn a_create_request_checks_the_name_before_the_project_id() {
        let err = CreatePipelineRequest {
            project_id: None,
            name: "  ".into(),
            nodes: Vec::new(),
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "Pipeline name cannot be empty");
    }

    #[test]
    fn a_get_or_delete_request_without_a_pipeline_id_names_the_wire_field() {
        let get = GetPipelineRequest { pipeline_id: None }
            .parse()
            .unwrap_err();
        let delete = DeletePipelineRequest { pipeline_id: None }
            .parse()
            .unwrap_err();

        assert_eq!(get.message(), "missing pipeline_id");
        assert_eq!(delete.message(), "missing pipeline_id");
    }

    #[test]
    fn a_run_request_without_a_pipeline_id_is_an_invalid_argument() {
        let err = RunPipelineRequest { pipeline_id: None }
            .parse()
            .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "missing pipeline_id");
    }
}
