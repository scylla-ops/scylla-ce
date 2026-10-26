use crate::domain::ids::TriggerId;
use crate::domain::job::Job;
use crate::domain::permission::Permission;
use scylla_extension::{Access, Command, Describe};

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
