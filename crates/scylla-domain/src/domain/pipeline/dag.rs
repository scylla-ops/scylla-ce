use crate::domain::pipeline::PipelineNode;
use std::collections::{BTreeSet, HashMap};

pub struct DagPlan<'a> {
    nodes: HashMap<&'a str, &'a PipelineNode>,
    in_degree: HashMap<&'a str, usize>,
    dependents: HashMap<&'a str, Vec<&'a str>>,
    ready: BTreeSet<&'a str>,
    pending: BTreeSet<&'a str>,
}

impl<'a> DagPlan<'a> {
    #[must_use]
    pub fn build(nodes: &'a [PipelineNode]) -> Self {
        let mut node_map: HashMap<&'a str, &'a PipelineNode> = HashMap::with_capacity(nodes.len());
        let mut in_degree: HashMap<&'a str, usize> = HashMap::with_capacity(nodes.len());
        let mut dependents: HashMap<&'a str, Vec<&'a str>> = HashMap::new();
        let mut pending: BTreeSet<&'a str> = BTreeSet::new();

        for node in nodes {
            let id = node.id().as_str();
            node_map.insert(id, node);
            in_degree.entry(id).or_insert(0);
            pending.insert(id);
        }

        for node in nodes {
            let id = node.id().as_str();
            for dep in node.deps() {
                *in_degree.entry(id).or_insert(0) += 1;
                dependents.entry(dep.as_str()).or_default().push(id);
            }
        }

        let ready: BTreeSet<&'a str> = in_degree
            .iter()
            .filter(|&(_, deg)| *deg == 0)
            .map(|(&id, _)| id)
            .collect();

        Self {
            nodes: node_map,
            in_degree,
            dependents,
            ready,
            pending,
        }
    }

    pub fn drain_ready(&mut self) -> Vec<&'a str> {
        let batch: Vec<&'a str> = self.ready.iter().copied().collect();
        self.ready.clear();
        batch
    }

    pub fn mark_completed(&mut self, id: &str) {
        self.pending.remove(id);
        if let Some(deps) = self.dependents.get(id) {
            for &dep_id in deps {
                if let Some(deg) = self.in_degree.get_mut(dep_id) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 && self.pending.contains(dep_id) {
                        self.ready.insert(dep_id);
                    }
                }
            }
        }
    }

    pub fn mark_terminal(&mut self, id: &str) {
        self.pending.remove(id);
        self.ready.remove(id);
    }

    pub fn pending(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.pending.iter().copied()
    }

    #[must_use]
    pub fn is_exhausted(&self) -> bool {
        self.pending.is_empty()
    }

    #[must_use]
    pub fn lookup(&self, id: &str) -> &'a PipelineNode {
        self.nodes[id]
    }

    /// Cycle check: a node whose in-degree never reaches zero is never drained.
    /// Duplicate ids and duplicate deps drain cleanly; `Pipeline::validate_nodes` rejects those.
    #[must_use]
    pub fn drains_completely(mut self) -> bool {
        loop {
            let batch = self.drain_ready();
            if batch.is_empty() {
                return self.is_exhausted();
            }
            for id in batch {
                self.mark_completed(id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::pipeline::{NodeId, Step};

    fn node(id: &str, deps: &[&str]) -> PipelineNode {
        PipelineNode::new(
            NodeId::new(id).unwrap(),
            deps.iter().map(|d| NodeId::new(*d).unwrap()).collect(),
            Step::exec("echo".into(), vec![]).unwrap(),
            None,
            vec![],
        )
    }

    #[test]
    fn initial_ready_is_roots() {
        let nodes = vec![node("a", &[]), node("b", &["a"]), node("c", &["a"])];
        let mut plan = DagPlan::build(&nodes);

        let batch = plan.drain_ready();
        assert_eq!(batch, vec!["a"]);
        assert!(!plan.is_exhausted());
    }

    #[test]
    fn mark_completed_unblocks_dependents() {
        let nodes = vec![node("a", &[]), node("b", &["a"]), node("c", &["a"])];
        let mut plan = DagPlan::build(&nodes);

        let _ = plan.drain_ready();
        plan.mark_completed("a");

        let mut batch = plan.drain_ready();
        batch.sort_unstable();
        assert_eq!(batch, vec!["b", "c"]);
    }

    #[test]
    fn diamond_order() {
        let nodes = vec![
            node("a", &[]),
            node("b", &["a"]),
            node("c", &["a"]),
            node("d", &["b", "c"]),
        ];
        let mut plan = DagPlan::build(&nodes);

        assert_eq!(plan.drain_ready(), vec!["a"]);
        plan.mark_completed("a");

        let mut batch = plan.drain_ready();
        batch.sort_unstable();
        assert_eq!(batch, vec!["b", "c"]);
        plan.mark_completed("b");
        assert!(plan.drain_ready().is_empty(), "d blocked by c");

        plan.mark_completed("c");
        assert_eq!(plan.drain_ready(), vec!["d"]);
        plan.mark_completed("d");

        assert!(plan.is_exhausted());
    }

    #[test]
    fn drains_completely_accepts_an_acyclic_graph() {
        let nodes = vec![
            node("a", &[]),
            node("b", &["a"]),
            node("c", &["a"]),
            node("d", &["b", "c"]),
        ];
        assert!(DagPlan::build(&nodes).drains_completely());
    }

    #[test]
    fn drains_completely_rejects_a_cycle() {
        let nodes = vec![node("a", &["c"]), node("b", &["a"]), node("c", &["b"])];
        assert!(!DagPlan::build(&nodes).drains_completely());
    }

    #[test]
    fn drains_completely_does_not_catch_structural_problems() {
        let duplicated_dep = vec![node("a", &[]), node("b", &["a", "a"])];
        assert!(DagPlan::build(&duplicated_dep).drains_completely());

        let duplicated_id = vec![node("a", &[]), node("a", &[])];
        assert!(DagPlan::build(&duplicated_id).drains_completely());
    }

    #[test]
    fn drains_completely_reports_unreachable_dependencies_as_a_cycle() {
        let dangling = vec![node("a", &[]), node("b", &["ghost"])];
        assert!(!DagPlan::build(&dangling).drains_completely());

        let self_dep = vec![node("a", &["a"])];
        assert!(!DagPlan::build(&self_dep).drains_completely());
    }

    #[test]
    fn drains_completely_rejects_a_cycle_hanging_off_a_valid_root() {
        let nodes = vec![
            node("root", &[]),
            node("x", &["root", "y"]),
            node("y", &["x"]),
        ];
        assert!(!DagPlan::build(&nodes).drains_completely());
    }

    #[test]
    fn pending_excludes_marked_terminal() {
        let nodes = vec![node("a", &[]), node("b", &["a"])];
        let mut plan = DagPlan::build(&nodes);

        plan.mark_terminal("a");
        let pending: Vec<&str> = plan.pending().collect();
        assert_eq!(pending, vec!["b"]);
    }

    #[test]
    fn mark_terminal_on_failure_leaves_dependents_pending() {
        let nodes = vec![node("a", &[]), node("b", &["a"])];
        let mut plan = DagPlan::build(&nodes);

        let _ = plan.drain_ready();
        plan.mark_terminal("a");

        assert_eq!(plan.drain_ready(), Vec::<&str>::new());
        let pending: Vec<&str> = plan.pending().collect();
        assert_eq!(pending, vec!["b"]);
    }
}
