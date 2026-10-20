use super::TriggerFireUseCases;
use crate::domain::errors::DomainResult;
use crate::domain::ids::TriggerId;
use crate::domain::job::Job;
use crate::domain::permission::Permission;
use async_trait::async_trait;
use scylla_extension::{
    Access, Authorized, Command, Committed, Describe, Persist, Prepare, Prepared, Run,
};

/// The caller's `runPipeline` on the trigger is the authorize stage; the fire then runs as the
/// trigger-runner App. The fire reads the trigger itself, so only the id is staged.
#[derive(Debug)]
pub struct FireTriggerNow {
    pub id: TriggerId,
}

impl Describe for FireTriggerNow {
    fn access(&self) -> Access {
        Access::Requires(Permission::RunTriggerPipeline(self.id.clone()))
    }
}

impl Command for FireTriggerNow {
    type Staged = TriggerId;
    type Committed = Job;
}

#[async_trait]
impl Run<Prepare<FireTriggerNow>> for TriggerFireUseCases {
    async fn run(
        &self,
        input: Authorized<FireTriggerNow>,
    ) -> DomainResult<Prepared<FireTriggerNow>> {
        let id = input.command().id.clone();
        Ok(input.prepared(id))
    }
}

#[async_trait]
impl Run<Persist<FireTriggerNow>> for TriggerFireUseCases {
    async fn run(
        &self,
        input: Prepared<FireTriggerNow>,
    ) -> DomainResult<Committed<FireTriggerNow>> {
        input
            .commit(async |id| self.firing.fire(&id, None, None).await)
            .await
    }
}
