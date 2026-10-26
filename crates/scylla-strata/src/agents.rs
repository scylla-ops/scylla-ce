use crate::access::CallerOf;
use scylla_auth::authz::{Grant, ORGANIZATION_AGENT_ROLE, Principal, Scope};
use scylla_core::application::agent::{
    AgentView, CreateAgent, CreatedAgent, GetAgent, GetAgentStats, ListAgents, RecordAgentHost,
    TouchAgent,
};
use scylla_core::application::app::mint_app_secret;
use scylla_core::application::guards::app_only;
use scylla_core::application::{AgentRepository, AgentStats, JobRepository};
use scylla_core::infrastructure::InMemoryAgentRegistry;
use scylla_domain::domain::agent::{Agent, AgentHost};
use scylla_domain::domain::app::{App, AppCredential, default_secret_label};
use scylla_domain::domain::clock;
use scylla_domain::domain::errors::DomainResult;
use scylla_domain::domain::ids::AppId;
use scylla_domain::domain::role::RoleName;
use scylla_extension::Describe;
use sqlx::types::chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use strata::{BuildCx, BuildError, Component, Resource};

#[derive(Clone)]
pub struct AgentStreams(pub Arc<InMemoryAgentRegistry>);

impl Resource for AgentStreams {
    const SECTION: &'static str = "agent_streams";
    async fn build(_: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Err(BuildError::msg(
            "the agent registry is built by the server: give it with `Builder::provide`",
        ))
    }
}

pub struct AgentConnections(Arc<InMemoryAgentRegistry>);

impl AgentConnections {
    #[must_use]
    pub fn registry(&self) -> &Arc<InMemoryAgentRegistry> {
        &self.0
    }
}

impl Component for AgentConnections {
    async fn build(cx: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Ok(AgentConnections(cx.resource::<AgentStreams>().await?.0))
    }
}

#[strata::module(section = "agents")]
pub trait Agents: crate::Apps {
    #[component]
    type AgentRepo = Arc<dyn AgentRepository>;

    #[component]
    type AgentJobs = Arc<dyn JobRepository>;

    #[component]
    type AgentLinks = AgentConnections;

    #[usecase(write, access = cmd.access())]
    async fn create_agent(&self, cmd: CreateAgent) -> DomainResult<CreatedAgent> {
        let app = App::create(cmd.organization_id.clone(), cmd.name.clone())?;
        let secret = mint_app_secret();
        let secret_hash = self.app_hash().hash_secret(&secret).await?;
        let label = cmd
            .secret_label
            .clone()
            .unwrap_or_else(|| default_secret_label(cmd.name.as_str()));
        let credential = AppCredential::create(app.id().clone(), label, secret_hash);
        let agent = Agent::create(app.id().clone());
        let grant = Grant::new(
            Principal::App(app.id().clone()),
            RoleName::new(ORGANIZATION_AGENT_ROLE)?,
            Scope::Organization(cmd.organization_id.clone()),
        );
        self.app_repo()
            .provision_agent(&app, &credential, &agent, &grant)
            .await?;
        Ok(CreatedAgent { app, secret })
    }

    #[usecase(write, access = cmd.access())]
    #[allow(unused_variables)]
    async fn touch_agent(&self, cmd: TouchAgent) -> DomainResult<DateTime<Utc>> {
        let app_id = app_only(self.ctx().caller())?;
        let at = clock::now();
        self.agent_repo().touch_last_seen(&app_id, at).await?;
        Ok(at)
    }

    #[usecase(write, access = cmd.access())]
    async fn record_agent_host(&self, cmd: RecordAgentHost) -> DomainResult<AgentHost> {
        let app_id = app_only(self.ctx().caller())?;
        self.agent_repo().record_host(&app_id, &cmd.host).await?;
        Ok(cmd.host)
    }

    #[usecase(read, access = query.access())]
    async fn list_agents(&self, query: ListAgents) -> DomainResult<Vec<AgentView>> {
        let agents = self
            .agent_repo()
            .list_by_organization(&query.organization_id)
            .await?;
        let connected: HashSet<AppId> = self
            .app_registry()
            .connected()
            .into_iter()
            .map(|stream| stream.agent)
            .collect();
        let ids: Vec<AppId> = agents.iter().map(|a| a.app_id().clone()).collect();
        let in_flight: HashMap<AppId, u32> = self
            .agent_jobs()
            .active_jobs(&ids)
            .await?
            .into_iter()
            .collect();
        let mut views = Vec::with_capacity(agents.len());
        for agent in &agents {
            let app = self.app_repo().find_by_id(agent.app_id()).await?;
            views.push(AgentView {
                connected: connected.contains(app.id()),
                in_flight: in_flight.get(app.id()).copied().unwrap_or(0),
                app,
                last_seen: agent.last_seen(),
                host: agent.host().cloned(),
            });
        }
        Ok(views)
    }

    #[usecase(read, access = query.access())]
    async fn get_agent(&self, query: GetAgent) -> DomainResult<AgentView> {
        let id = &query.id;
        let app = self.app_repo().find_by_id(id).await?;
        let agent = self.agent_repo().find_by_app_id(id).await?;
        let in_flight = self
            .agent_jobs()
            .active_jobs(std::slice::from_ref(id))
            .await?
            .into_iter()
            .map(|(_, count)| count)
            .sum();
        Ok(AgentView {
            app,
            connected: self
                .app_registry()
                .connected()
                .iter()
                .any(|s| &s.agent == id),
            last_seen: agent.last_seen(),
            in_flight,
            host: agent.host().cloned(),
        })
    }

    #[usecase(read, access = query.access())]
    async fn get_agent_stats(&self, query: GetAgentStats) -> DomainResult<AgentStats> {
        let app = self.app_repo().find_by_id(&query.id).await?;
        self.agent_repo().agent_stats(app.id()).await
    }
}
