//! The agent's reads. One block per query, in the order it runs: the struct, its access, its
//! output type, what `Fetch` reads.

use super::AgentUseCases;
use crate::application::AgentStats;
use crate::domain::agent::AgentHost;
use crate::domain::app::App;
use crate::domain::errors::DomainResult;
use crate::domain::ids::{AppId, OrganizationId};
use crate::domain::permission::Permission;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scylla_extension::{Access, Authorized, Describe, Fetch, Fetched, Query, Run};
use std::collections::{HashMap, HashSet};

/// `connected` comes from the registry, `in_flight` from the jobs placed on the agent.
pub struct AgentView {
    pub app: App,
    pub connected: bool,
    pub last_seen: Option<DateTime<Utc>>,
    pub in_flight: u32,
    pub host: Option<AgentHost>,
}

#[derive(Debug)]
pub struct ListAgents {
    pub organization_id: OrganizationId,
}

impl Describe for ListAgents {
    fn access(&self) -> Access {
        Access::Requires(Permission::ListAgents(self.organization_id.clone()))
    }
}

impl Query for ListAgents {
    type Output = Vec<AgentView>;
}

#[async_trait]
impl Run<Fetch<ListAgents>> for AgentUseCases {
    async fn run(&self, input: Authorized<ListAgents>) -> DomainResult<Fetched<ListAgents>> {
        let agents = self
            .agent_repo
            .list_by_organization(&input.command().organization_id)
            .await?;
        let connected: HashSet<AppId> = self
            .registry
            .connected()
            .into_iter()
            .map(|stream| stream.agent)
            .collect();
        let ids: Vec<AppId> = agents.iter().map(|a| a.app_id().clone()).collect();
        let in_flight: HashMap<AppId, u32> =
            self.job_repo.active_jobs(&ids).await?.into_iter().collect();

        let mut views = Vec::with_capacity(agents.len());
        for agent in &agents {
            let app = self.app_repo.find_by_id(agent.app_id()).await?;
            views.push(AgentView {
                connected: connected.contains(app.id()),
                in_flight: in_flight.get(app.id()).copied().unwrap_or(0),
                app,
                last_seen: agent.last_seen(),
                host: agent.host().cloned(),
            });
        }
        Ok(input.fetched(views))
    }
}

#[derive(Debug)]
pub struct GetAgent {
    pub id: AppId,
}

impl Describe for GetAgent {
    fn access(&self) -> Access {
        Access::Requires(Permission::ReadApp(self.id.clone()))
    }
}

impl Query for GetAgent {
    type Output = AgentView;
}

#[async_trait]
impl Run<Fetch<GetAgent>> for AgentUseCases {
    async fn run(&self, input: Authorized<GetAgent>) -> DomainResult<Fetched<GetAgent>> {
        let id = &input.command().id;
        let app = self.app_repo.find_by_id(id).await?;
        let agent = self.agent_repo.find_by_app_id(id).await?;
        let in_flight = self
            .job_repo
            .active_jobs(std::slice::from_ref(id))
            .await?
            .into_iter()
            .map(|(_, count)| count)
            .sum();
        let view = AgentView {
            app,
            connected: self.registry.connected().iter().any(|s| &s.agent == id),
            last_seen: agent.last_seen(),
            in_flight,
            host: agent.host().cloned(),
        };
        Ok(input.fetched(view))
    }
}

#[derive(Debug)]
pub struct GetAgentStats {
    pub id: AppId,
}

impl Describe for GetAgentStats {
    fn access(&self) -> Access {
        Access::Requires(Permission::ReadAppStats(self.id.clone()))
    }
}

impl Query for GetAgentStats {
    type Output = AgentStats;
}

#[async_trait]
impl Run<Fetch<GetAgentStats>> for AgentUseCases {
    async fn run(&self, input: Authorized<GetAgentStats>) -> DomainResult<Fetched<GetAgentStats>> {
        let app = self.app_repo.find_by_id(&input.command().id).await?;
        let stats = self.agent_repo.agent_stats(app.id()).await?;
        Ok(input.fetched(stats))
    }
}
