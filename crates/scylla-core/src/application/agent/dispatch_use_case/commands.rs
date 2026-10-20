use super::DispatchUseCases;
use crate::application::actions::service_only;
use crate::application::agent::AgentStream;
use crate::domain::caller::CallerContext;
use crate::domain::errors::DomainResult;
use crate::domain::ids::{AppId, PipelineId};
use crate::domain::job::Job;
use crate::domain::permission::Permission;
use async_trait::async_trait;
use scylla_auth::authz::Visibility;
use scylla_extension::{
    Access, Authorized, Command, Committed, Describe, Persist, Prepare, Prepared, Run,
};
use std::collections::HashSet;

/// One pass: each targeted agent that is connected, idle and holds `executeJob` somewhere gets
/// the oldest pending job it may run, on its open stream. `None` targets every connected agent.
/// `Committed` is the jobs an agent took.
#[derive(Debug)]
pub struct DispatchPendingJobs {
    pub agents: Option<Vec<AppId>>,
}

impl Describe for DispatchPendingJobs {
    fn access(&self) -> Access {
        Access::Authenticated
    }
}

impl Command for DispatchPendingJobs {
    type Staged = Vec<(AgentStream, Visibility)>;
    type Committed = Vec<Job>;
}

#[async_trait]
impl Run<Prepare<DispatchPendingJobs>> for DispatchUseCases {
    async fn run(
        &self,
        input: Authorized<DispatchPendingJobs>,
    ) -> DomainResult<Prepared<DispatchPendingJobs>> {
        service_only(input.caller())?;
        let mut targets = self.registry.connected();
        if let Some(agents) = &input.command().agents {
            targets.retain(|stream| agents.contains(&stream.agent));
        }
        let agents: Vec<AppId> = targets.iter().map(|stream| stream.agent.clone()).collect();
        let busy: HashSet<AppId> = self
            .job_repo
            .active_jobs(&agents)
            .await?
            .into_iter()
            .map(|(agent, _)| agent)
            .collect();
        let execute = Permission::ExecuteJob(PipelineId::new("_"));
        let mut staged = Vec::new();
        for stream in targets.into_iter().filter(|s| !busy.contains(&s.agent)) {
            let visible = self
                .visibility
                .visible_scopes(&CallerContext::App(stream.agent.clone()), execute.key())
                .await?;
            if !visible.is_empty() {
                staged.push((stream, visible));
            }
        }
        Ok(input.prepared(staged))
    }
}

#[async_trait]
impl Run<Persist<DispatchPendingJobs>> for DispatchUseCases {
    async fn run(
        &self,
        input: Prepared<DispatchPendingJobs>,
    ) -> DomainResult<Committed<DispatchPendingJobs>> {
        input
            .commit(async |staged| {
                let mut placed = Vec::new();
                for (stream, visible) in staged {
                    placed.extend(self.place(&stream, &visible).await?);
                }
                Ok(placed)
            })
            .await
    }
}
