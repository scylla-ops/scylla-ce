//! The trigger's reads. Each query, its access and its output type. The strata module of the slice
//! runs it.

use crate::domain::errors::DomainResult;
use crate::domain::ids::{AppId, PipelineId, TriggerId};
use crate::domain::permission::Permission;
use crate::domain::trigger::Trigger;
use scylla_extension::{Access, Describe, Query};

#[derive(Debug)]
pub struct GetTrigger {
    pub id: TriggerId,
}

impl Describe for GetTrigger {
    fn access(&self) -> Access {
        Access::Requires(Permission::ManageTrigger(self.id.clone()))
    }
}

impl Query for GetTrigger {
    type Output = Trigger;
}

#[derive(Debug)]
pub struct ListPipelineTriggers {
    pub pipeline_id: PipelineId,
}

impl Describe for ListPipelineTriggers {
    fn access(&self) -> Access {
        Access::Requires(Permission::ManageTriggers(self.pipeline_id.clone()))
    }
}

impl Query for ListPipelineTriggers {
    type Output = Vec<Trigger>;
}

/// What a fire needs, read by the trigger firer service: the trigger, refused when it is
/// disabled, and the trigger-runner App of its organization. A runner that cannot be found is a
/// failed run, not a refusal, so the fire still records it.
#[derive(Debug)]
pub struct ResolveTriggerRun {
    pub id: TriggerId,
}

pub struct TriggerRun {
    pub trigger: Trigger,
    pub runner: DomainResult<AppId>,
}

impl Describe for ResolveTriggerRun {
    fn access(&self) -> Access {
        Access::Authenticated
    }
}

impl Query for ResolveTriggerRun {
    type Output = TriggerRun;
}
