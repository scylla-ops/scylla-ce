//! The rules on the provider that block the merge into the default branch when a pipeline fails:
//! one for each pipeline on GitHub, one for the project on GitLab.

use time::OffsetDateTime;

use super::named;
use crate::identity::actors::ActorId;
use crate::projects::pipelines::PipelineId;
use crate::projects::ProjectId;

named! {
    /// The state of a merge rule on the provider.
    pub enum MergeBlockState {
        /// The rule is off.
        Off = "off",
        /// Scylla changes the rule on the provider.
        Applying = "applying",
        /// The rule is on.
        On = "on",
        /// The connection misses the permission or the role: a person grants it, or sets the
        /// rule by hand.
        NeedsPermission = "needs_permission",
        /// The provider does not allow the rule, for example on a private repository of a free
        /// GitHub account.
        NotPossible = "not_possible",
        /// A person changed the rule on the provider.
        OutOfSync = "out_of_sync",
        /// The last change on the provider failed.
        Failed = "failed",
    }
}

/// A merge rule, as the store returns it.
#[derive(Clone, Debug)]
pub struct MergeBlock {
    /// The project.
    pub project_id: ProjectId,
    /// The pipeline, or `None` for the rule of the project on GitLab.
    pub pipeline_id: Option<PipelineId>,
    /// The name of the report that the rule needs, for example `scylla/build`, or `None` for the
    /// rule of the project.
    pub context: Option<String>,
    /// `true` when the rule must be on.
    pub enabled: bool,
    /// The state on the provider.
    pub state: MergeBlockState,
    /// The reason of a state that needs a person, as a stable code.
    pub state_reason: Option<String>,
    /// The actor of the last change.
    pub updated_by: ActorId,
    /// The time of the last change.
    pub updated_at: OffsetDateTime,
}
