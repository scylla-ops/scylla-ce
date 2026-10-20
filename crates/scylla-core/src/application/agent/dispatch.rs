use crate::application::secret::SecretResolver;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::ProjectId;
use crate::domain::job::Job;
use crate::domain::pipeline::{EnvKey, EnvValue, Step};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Half the frame limit of the agent stream, so the encoding overhead never crosses it.
pub const MAX_DISPATCH_BYTES: usize = scylla_proto::agent::MAX_MESSAGE_BYTES / 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDispatch {
    pub job_id: String,
    pub pipeline_id: String,
    pub nodes: Vec<DispatchNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchNode {
    pub id: String,
    pub deps: Vec<String>,
    pub working_dir: Option<String>,
    pub step: Step,
    pub env: Vec<DispatchEnv>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchEnv {
    pub key: String,
    pub value: String,
    pub masked: bool,
}

impl JobDispatch {
    /// The bytes of every string the agent receives.
    #[must_use]
    pub fn size(&self) -> usize {
        self.job_id.len()
            + self.pipeline_id.len()
            + self.nodes.iter().map(DispatchNode::size).sum::<usize>()
    }
}

impl DispatchNode {
    fn size(&self) -> usize {
        let step = match &self.step {
            Step::Exec { command, args } => {
                command.as_str().len() + args.iter().map(|a| a.as_str().len()).sum::<usize>()
            }
            Step::Script { script, .. } => script.as_str().len(),
        };
        self.id.len()
            + self.deps.iter().map(String::len).sum::<usize>()
            + self.working_dir.as_ref().map_or(0, String::len)
            + step
            + self
                .env
                .iter()
                .map(|e| e.key.len() + e.value.len())
                .sum::<usize>()
    }
}

/// The run checks and the dispatcher sends the same dispatch: the nodes the job was created
/// with, their secrets resolved, the trigger inputs added.
pub async fn assemble_dispatch(
    secret_resolver: &dyn SecretResolver,
    project_id: &ProjectId,
    job: &Job,
) -> DomainResult<JobDispatch> {
    let nodes = secret_resolver.resolve(project_id, job.nodes()).await?;
    let dispatch = JobDispatch {
        job_id: job.id().to_string(),
        pipeline_id: job.pipeline_id().to_string(),
        nodes: apply_inputs(nodes, job.inputs()),
    };
    let size = dispatch.size();
    if size > MAX_DISPATCH_BYTES {
        return Err(DomainError::validation(format!(
            "the resolved job is {size} bytes; the limit is {MAX_DISPATCH_BYTES}"
        )));
    }
    Ok(dispatch)
}

/// A node's own env wins on a key collision: a trigger adds context, never overrides.
fn apply_inputs(mut nodes: Vec<DispatchNode>, inputs: &[(EnvKey, EnvValue)]) -> Vec<DispatchNode> {
    if inputs.is_empty() {
        return nodes;
    }
    for node in &mut nodes {
        let existing: HashSet<String> = node.env.iter().map(|e| e.key.clone()).collect();
        for (key, value) in inputs {
            if !existing.contains(key.as_str()) {
                node.env.push(DispatchEnv {
                    key: key.to_string(),
                    value: value.to_string(),
                    masked: false,
                });
            }
        }
    }
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::ProjectId;
    use crate::domain::pipeline::{NodeId, PipelineNode, Shell};
    use crate::test_support::jobs::job;
    use crate::test_support::pipelines::{PipelineBuilder, node as pipeline_node};
    use crate::test_support::stubs::EchoResolver;

    fn script_node(id: &str, body: String) -> PipelineNode {
        PipelineNode::new(
            NodeId::new(id).unwrap(),
            vec![],
            Step::script(body, Shell::Sh).unwrap(),
            None,
            vec![],
        )
    }

    #[tokio::test]
    async fn a_dispatch_carries_the_nodes_the_job_was_created_with() {
        let mut pipeline = PipelineBuilder::for_project_id(ProjectId::new("p"))
            .nodes(vec![
                pipeline_node("old1", &[]),
                pipeline_node("old2", &["old1"]),
            ])
            .build();
        let job = job(&pipeline);
        pipeline
            .update_nodes(vec![pipeline_node("new", &[])])
            .unwrap();

        let dispatch = assemble_dispatch(&EchoResolver, pipeline.project_id(), &job)
            .await
            .unwrap();

        let ids: Vec<&str> = dispatch.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["old1", "old2"]);
        assert_eq!(dispatch.job_id, job.id().to_string());
        assert_eq!(dispatch.pipeline_id, pipeline.id().to_string());
    }

    #[tokio::test]
    async fn a_job_larger_than_the_dispatch_limit_is_refused() {
        let script = "x".repeat(1024 * 1024);
        let nodes = (0..9)
            .map(|i| script_node(&format!("n{i}"), script.clone()))
            .collect();
        let pipeline = PipelineBuilder::for_project_id(ProjectId::new("p"))
            .nodes(nodes)
            .build();

        let err = assemble_dispatch(&EchoResolver, pipeline.project_id(), &job(&pipeline))
            .await
            .unwrap_err();

        assert!(matches!(err, DomainError::Validation(_)));
    }

    fn node(env: &[(&str, &str)]) -> DispatchNode {
        DispatchNode {
            id: "n".to_string(),
            deps: vec![],
            working_dir: None,
            step: Step::exec("echo".to_string(), vec![]).unwrap(),
            env: env
                .iter()
                .map(|(k, v)| DispatchEnv {
                    key: (*k).to_string(),
                    value: (*v).to_string(),
                    masked: false,
                })
                .collect(),
        }
    }

    fn env_of(node: &DispatchNode) -> Vec<(String, String)> {
        node.env
            .iter()
            .map(|e| (e.key.clone(), e.value.clone()))
            .collect()
    }

    fn input(key: &str, value: &str) -> (EnvKey, EnvValue) {
        (EnvKey::new(key).unwrap(), EnvValue::new(value).unwrap())
    }

    #[test]
    fn empty_inputs_leave_nodes_untouched() {
        let nodes = vec![node(&[("A", "1")])];
        let out = apply_inputs(nodes, &[]);
        assert_eq!(env_of(&out[0]), vec![("A".into(), "1".into())]);
    }

    #[test]
    fn inputs_are_appended_as_literals() {
        let out = apply_inputs(vec![node(&[])], &[input("GIT_COMMIT", "abc")]);
        assert_eq!(env_of(&out[0]), vec![("GIT_COMMIT".into(), "abc".into())]);
        assert!(!out[0].env[0].masked);
    }

    #[test]
    fn node_env_wins_on_collision() {
        let out = apply_inputs(
            vec![node(&[("GIT_COMMIT", "from-node")])],
            &[input("GIT_COMMIT", "from-input")],
        );
        assert_eq!(out[0].env.len(), 1);
        assert_eq!(out[0].env[0].value, "from-node");
    }
}
