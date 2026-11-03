//! The plugin of the pipelines and of their jobs.

use scylla_domain::projects::pipelines::{PipelineAccess, PipelineHolder};
use scylla_extension::{EditionBuilder, Plugin};

/// Adds the permission model of the pipelines and of the jobs, and the role `runner` of the
/// agents in a workspace. The services need the open streams of the agents, so the server adds
/// them. The tables come from the core migrations.
pub(crate) struct Pipelines;

impl Plugin for Pipelines {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition
            .authz::<PipelineAccess>()
            .authz::<PipelineHolder>()
            .role(PipelineHolder::RUNNER)
    }
}
