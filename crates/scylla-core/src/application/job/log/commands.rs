//! The job log's writes. One block per command, in the order it runs: the struct, its
//! access, its payload types, what `Prepare` builds, what `Persist` writes.

use super::JobLogUseCases;
use crate::application::actions::app_only;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::JobId;
use crate::domain::job::JobLog;
use crate::domain::permission::Permission;
use async_trait::async_trait;
use scylla_extension::{
    Access, Authorized, Command, Committed, Describe, Draft, Persist, Prepare, Prepared, Run,
};

/// Consecutive lines of one job from its agent, in the order received: one check and one
/// insert for the batch. Only the agent the job is placed on appends, until the job ends.
#[derive(Debug)]
pub struct AppendJobLogs {
    pub job_id: JobId,
    pub logs: Vec<JobLog>,
}

impl Describe for AppendJobLogs {
    fn access(&self) -> Access {
        Access::Requires(Permission::AppendJobLog(self.job_id.clone()))
    }
}

impl Command for AppendJobLogs {
    type Staged = Draft<Vec<JobLog>>;
    type Committed = Vec<JobLog>;
}

#[async_trait]
impl Run<Prepare<AppendJobLogs>> for JobLogUseCases {
    async fn run(&self, input: Authorized<AppendJobLogs>) -> DomainResult<Prepared<AppendJobLogs>> {
        let agent = app_only(input.caller())?;
        let cmd = input.command();
        if cmd.logs.iter().any(|log| log.job_id() != &cmd.job_id) {
            return Err(DomainError::validation("every line must belong to the job"));
        }
        self.job_repo
            .find_by_id(&cmd.job_id)
            .await?
            .ensure_live_on(&agent)?;
        let logs = cmd.logs.clone();
        Ok(input.prepared(Draft::new(logs)))
    }
}

#[async_trait]
impl Run<Persist<AppendJobLogs>> for JobLogUseCases {
    async fn run(&self, input: Prepared<AppendJobLogs>) -> DomainResult<Committed<AppendJobLogs>> {
        input
            .commit(async |draft| {
                let logs = draft.into_inner();
                self.log_repo.create_many(&logs).await?;
                for log in &logs {
                    self.stream_port.publish(log);
                }
                Ok(logs)
            })
            .await
    }
}
