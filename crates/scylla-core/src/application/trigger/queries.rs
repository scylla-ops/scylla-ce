//! The trigger's reads. One block per query, in the order it runs: the struct, its access,
//! its output type, what `Fetch` reads.

use super::TriggerUseCases;
use crate::application::actions::service_only;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{AppId, PipelineId, TriggerId};
use crate::domain::permission::Permission;
use crate::domain::trigger::Trigger;
use async_trait::async_trait;
use scylla_extension::{Access, Authorized, Describe, Fetch, Fetched, Query, Run};

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

#[async_trait]
impl Run<Fetch<GetTrigger>> for TriggerUseCases {
    async fn run(&self, input: Authorized<GetTrigger>) -> DomainResult<Fetched<GetTrigger>> {
        let trigger = self.trigger_repo.find_by_id(&input.command().id).await?;
        Ok(input.fetched(trigger))
    }
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

#[async_trait]
impl Run<Fetch<ListPipelineTriggers>> for TriggerUseCases {
    async fn run(
        &self,
        input: Authorized<ListPipelineTriggers>,
    ) -> DomainResult<Fetched<ListPipelineTriggers>> {
        let triggers = self
            .trigger_repo
            .list_by_pipeline(&input.command().pipeline_id)
            .await?;
        Ok(input.fetched(triggers))
    }
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

#[async_trait]
impl Run<Fetch<ResolveTriggerRun>> for TriggerUseCases {
    async fn run(
        &self,
        input: Authorized<ResolveTriggerRun>,
    ) -> DomainResult<Fetched<ResolveTriggerRun>> {
        service_only(input.caller())?;
        let trigger = self.trigger_repo.find_by_id(&input.command().id).await?;
        if !trigger.is_enabled() {
            return Err(DomainError::business_rule("trigger is disabled"));
        }
        let runner = self.trigger_runner(&trigger).await;
        Ok(input.fetched(TriggerRun { trigger, runner }))
    }
}
