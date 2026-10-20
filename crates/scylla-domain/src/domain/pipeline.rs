mod dag;
mod env;
mod name;
mod node_id;
mod step;
mod working_dir;

pub use dag::*;
pub use env::*;
pub use name::*;
pub use node_id::*;
pub use step::*;
pub use working_dir::*;

use crate::domain::clock;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{PipelineId, ProjectId};
use chrono::{DateTime, Utc};

use std::collections::HashSet;

pub const MAX_NODES: usize = 256;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PipelineNode {
    id: NodeId,
    deps: Vec<NodeId>,
    #[serde(default)]
    working_dir: Option<WorkingDir>,
    #[serde(default)]
    env: Vec<EnvVar>,
    step: Step,
}

impl PipelineNode {
    #[must_use]
    pub fn new(
        id: NodeId,
        deps: Vec<NodeId>,
        step: Step,
        working_dir: Option<WorkingDir>,
        env: Vec<EnvVar>,
    ) -> Self {
        Self {
            id,
            deps,
            working_dir,
            env,
            step,
        }
    }

    #[must_use]
    pub fn id(&self) -> &NodeId {
        &self.id
    }

    #[must_use]
    pub fn deps(&self) -> &[NodeId] {
        &self.deps
    }

    #[must_use]
    pub fn step(&self) -> &Step {
        &self.step
    }

    #[must_use]
    pub fn working_dir(&self) -> Option<&WorkingDir> {
        self.working_dir.as_ref()
    }

    #[must_use]
    pub fn env(&self) -> &[EnvVar] {
        &self.env
    }

    fn validate(&self, ids: &HashSet<&NodeId>) -> DomainResult<()> {
        let mut seen_deps = HashSet::new();
        for dep_id in &self.deps {
            if dep_id == &self.id {
                return Err(DomainError::validation(format!(
                    "Node '{}' cannot depend on itself",
                    self.id
                )));
            }
            if !ids.contains(dep_id) {
                return Err(DomainError::validation(format!(
                    "Node '{}' has invalid dependency: {}",
                    self.id, dep_id
                )));
            }
            if !seen_deps.insert(dep_id) {
                return Err(DomainError::validation(format!(
                    "Node '{}' has duplicate dependency: {}",
                    self.id, dep_id
                )));
            }
        }

        if self.env.len() > MAX_ENV_VARS {
            return Err(DomainError::validation(format!(
                "Node '{}' cannot have more than {MAX_ENV_VARS} env vars",
                self.id
            )));
        }
        let mut seen_keys = HashSet::new();
        for key in self.env.iter().map(EnvVar::key) {
            if !seen_keys.insert(key) {
                return Err(DomainError::validation(format!(
                    "Node '{}' has duplicate env key: {}",
                    self.id, key
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Pipeline {
    id: PipelineId,
    project_id: ProjectId,
    name: PipelineName,
    nodes: Vec<PipelineNode>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    /// The row version the value was read at. The store checks it on every write and bumps
    /// it itself; the domain never changes it.
    version: u64,
}

impl Pipeline {
    #[must_use]
    pub fn from_persistence(
        id: PipelineId,
        project_id: ProjectId,
        name: PipelineName,
        nodes: Vec<PipelineNode>,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        version: u64,
    ) -> Self {
        Self {
            id,
            project_id,
            name,
            nodes,
            created_at,
            updated_at,
            version,
        }
    }

    pub fn create(
        name: PipelineName,
        project_id: ProjectId,
        nodes: Vec<PipelineNode>,
    ) -> DomainResult<Self> {
        Self::validate_nodes(&nodes)?;

        let now = clock::now();
        Ok(Pipeline {
            id: PipelineId::generate(),
            project_id,
            name,
            nodes,
            created_at: now,
            updated_at: now,
            version: 0,
        })
    }

    pub fn update_name(&mut self, name: PipelineName) -> DomainResult<()> {
        self.name = name;
        self.updated_at = clock::now();
        Ok(())
    }

    pub fn update_nodes(&mut self, nodes: Vec<PipelineNode>) -> DomainResult<()> {
        Self::validate_nodes(&nodes)?;
        self.nodes = nodes;
        self.updated_at = clock::now();
        Ok(())
    }

    fn validate_nodes(nodes: &[PipelineNode]) -> DomainResult<()> {
        if nodes.is_empty() {
            return Err(DomainError::validation(
                "Pipeline must have at least one node",
            ));
        }

        if nodes.len() > MAX_NODES {
            return Err(DomainError::validation(format!(
                "Pipeline cannot have more than {MAX_NODES} nodes"
            )));
        }

        let mut node_ids = HashSet::new();
        for node in nodes {
            if !node_ids.insert(node.id()) {
                return Err(DomainError::validation(format!(
                    "Duplicate node ID: {}",
                    node.id()
                )));
            }
        }

        for node in nodes {
            node.validate(&node_ids)?;
        }

        // Cycle check last: dangling and self deps never drain either, and the checks above own their errors.
        // Duplicate ids and duplicate deps drain cleanly, so the checks above are their only rejection.
        if !DagPlan::build(nodes).drains_completely() {
            return Err(DomainError::business_rule("Cycle detected in pipeline"));
        }

        Ok(())
    }

    #[must_use]
    pub fn id(&self) -> &PipelineId {
        &self.id
    }

    #[must_use]
    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    #[must_use]
    pub fn name(&self) -> &PipelineName {
        &self.name
    }

    #[must_use]
    pub fn nodes(&self) -> &[PipelineNode] {
        &self.nodes
    }

    #[must_use]
    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    #[must_use]
    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }

    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::pipeline::Step;
    use crate::domain::secret::SecretName;

    fn node_id(s: &str) -> NodeId {
        NodeId::new(s).unwrap()
    }

    fn pipeline_name() -> PipelineName {
        PipelineName::new("test-pipeline").unwrap()
    }

    fn project_id() -> ProjectId {
        ProjectId::generate()
    }

    fn action(id: &str, deps: &[&str]) -> PipelineNode {
        PipelineNode::new(
            node_id(id),
            deps.iter().map(|d| node_id(d)).collect(),
            Step::exec("echo".into(), vec![]).unwrap(),
            None,
            vec![],
        )
    }

    #[test]
    fn valid_dag() {
        let nodes = vec![
            action("a", &[]),
            action("b", &["a"]),
            action("c", &["a", "b"]),
        ];
        let pipeline = Pipeline::create(pipeline_name(), project_id(), nodes).unwrap();
        assert_eq!(pipeline.nodes().len(), 3);
    }

    fn create_err(nodes: Vec<PipelineNode>) -> DomainError {
        Pipeline::create(pipeline_name(), project_id(), nodes).unwrap_err()
    }

    // The variant matters: Validation maps to InvalidArgument, BusinessRule to FailedPrecondition.

    #[test]
    fn rejects_cycle() {
        let nodes = vec![
            action("a", &["c"]),
            action("b", &["a"]),
            action("c", &["b"]),
        ];
        assert!(matches!(create_err(nodes), DomainError::BusinessRule(_)));
    }

    #[test]
    fn rejects_invalid_dependency() {
        let nodes = vec![action("a", &["nonexistent"])];
        assert!(matches!(create_err(nodes), DomainError::Validation(_)));
    }

    #[test]
    fn rejects_duplicate_ids() {
        let nodes = vec![action("a", &[]), action("a", &[])];
        assert!(matches!(create_err(nodes), DomainError::Validation(_)));
    }

    #[test]
    fn rejects_duplicate_deps() {
        let nodes = vec![action("a", &[]), action("b", &["a", "a"])];
        assert!(matches!(create_err(nodes), DomainError::Validation(_)));
    }

    fn with_env(id: &str, env: Vec<EnvVar>) -> PipelineNode {
        PipelineNode::new(
            node_id(id),
            vec![],
            Step::exec("env".into(), vec![]).unwrap(),
            None,
            env,
        )
    }

    fn env_key(k: &str) -> EnvKey {
        EnvKey::new(k).unwrap()
    }

    #[test]
    fn rejects_too_many_nodes() {
        let nodes = |n: usize| (0..n).map(|i| action(&format!("n{i}"), &[])).collect();
        assert!(Pipeline::create(pipeline_name(), project_id(), nodes(MAX_NODES)).is_ok());
        assert!(matches!(
            create_err(nodes(MAX_NODES + 1)),
            DomainError::Validation(_)
        ));
    }

    #[test]
    fn rejects_too_many_env_vars() {
        let env = |n: usize| {
            (0..n)
                .map(|i| EnvVar::literal(env_key(&format!("K{i}")), "v").unwrap())
                .collect()
        };
        assert!(
            Pipeline::create(
                pipeline_name(),
                project_id(),
                vec![with_env("a", env(MAX_ENV_VARS))]
            )
            .is_ok()
        );
        assert!(matches!(
            create_err(vec![with_env("a", env(MAX_ENV_VARS + 1))]),
            DomainError::Validation(_)
        ));
    }

    #[test]
    fn rejects_duplicate_env_keys() {
        let secret = SecretName::new("TOKEN").unwrap();
        for env in [
            vec![
                EnvVar::literal(env_key("K"), "1").unwrap(),
                EnvVar::literal(env_key("K"), "2").unwrap(),
            ],
            vec![
                EnvVar::literal(env_key(" K "), "1").unwrap(),
                EnvVar::literal(env_key("K"), "2").unwrap(),
            ],
            vec![
                EnvVar::literal(env_key("X"), "literal").unwrap(),
                EnvVar::secret(env_key("X"), secret.clone()),
            ],
        ] {
            let err = create_err(vec![with_env("a", env)]);
            assert!(
                matches!(&err, DomainError::Validation(m) if m.contains("duplicate env key")),
                "{err}"
            );
        }
    }

    #[test]
    fn update_nodes_preserves_on_failure() {
        let mut pipeline =
            Pipeline::create(pipeline_name(), project_id(), vec![action("a", &[])]).unwrap();

        let cyclic = vec![action("x", &["y"]), action("y", &["x"])];
        assert!(pipeline.update_nodes(cyclic).is_err());
        assert_eq!(pipeline.nodes().len(), 1);
    }

    #[test]
    fn update_nodes_replaces_the_whole_set() {
        let mut pipeline =
            Pipeline::create(pipeline_name(), project_id(), vec![action("a", &[])]).unwrap();

        let new_nodes = vec![action("x", &[]), action("y", &["x"])];
        pipeline.update_nodes(new_nodes).unwrap();

        assert_eq!(pipeline.nodes().len(), 2);
        let ids: Vec<&NodeId> = pipeline.nodes().iter().map(PipelineNode::id).collect();
        assert_eq!(ids, [&node_id("x"), &node_id("y")]);
    }

    #[test]
    fn rejects_empty_action_command() {
        assert!(Step::exec("   ".into(), vec![]).is_err());
    }

    /// Golden: the on-disk shape of `pipelines.nodes`; change only with a migration.
    #[test]
    fn pipeline_nodes_jsonb_shape_is_stable() {
        const STORED: &str = r#"[
            {"id":"build","deps":[],"working_dir":"crates/api",
             "env":[{"key":"RUST_LOG","source":{"literal":"debug"}}],
             "step":{"kind":"exec","command":"cargo","args":["build"]}},
            {"id":"test","deps":["build"],"working_dir":null,"env":[],
             "step":{"kind":"script","script":"cargo test\n","shell":"bash"}}
        ]"#;

        let nodes: Vec<PipelineNode> = serde_json::from_str(STORED).unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].id().as_str(), "build");
        assert_eq!(nodes[0].working_dir().unwrap().as_str(), "crates/api");
        assert_eq!(nodes[0].env().len(), 1);
        assert!(nodes[1].working_dir().is_none());
        assert_eq!(nodes[1].deps(), &[node_id("build")]);

        let round_tripped: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&nodes).unwrap()).unwrap();
        let original: serde_json::Value = serde_json::from_str(STORED).unwrap();
        assert_eq!(round_tripped, original, "serialized shape drifted");
    }

    #[test]
    fn pipeline_nodes_jsonb_tolerates_missing_optional_keys() {
        const LEGACY: &str =
            r#"[{"id":"a","deps":[],"step":{"kind":"script","script":"ls","shell":"sh"}}]"#;

        let nodes: Vec<PipelineNode> = serde_json::from_str(LEGACY).unwrap();
        assert!(nodes[0].working_dir().is_none());
        assert!(nodes[0].env().is_empty());
    }
}
