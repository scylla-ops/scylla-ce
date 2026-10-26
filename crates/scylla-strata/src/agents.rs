use crate::access::CallerOf;
use scylla_auth::authz::{Grant, ORGANIZATION_AGENT_ROLE, Principal, Scope};
use scylla_core::application::agent::{AgentView, CreatedAgent};
use scylla_core::application::app::mint_app_secret;
use scylla_core::application::guards::app_only;
use scylla_core::application::{AgentRepository, AgentStats, JobRepository};
use scylla_core::infrastructure::InMemoryAgentRegistry;
use scylla_domain::domain::agent::{Agent, AgentHost};
use scylla_domain::domain::app::{
    App, AppCredential, AppName, AppSecretLabel, default_secret_label,
};
use scylla_domain::domain::clock;
use scylla_domain::domain::errors::DomainResult;
use scylla_domain::domain::ids::{AppId, OrganizationId};
use scylla_domain::domain::permission::Permission;
use scylla_domain::domain::role::RoleName;
use scylla_extension::Access;
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

    #[usecase(write, access = Access::Requires(Permission::CreateAgent(organization_id.clone())))]
    async fn create_agent(
        &self,
        organization_id: OrganizationId,
        name: AppName,
        secret_label: Option<AppSecretLabel>,
    ) -> DomainResult<CreatedAgent> {
        let app = App::create(organization_id.clone(), name.clone())?;
        let secret = mint_app_secret();
        let secret_hash = self.app_hash().hash_secret(&secret).await?;
        let label = secret_label.unwrap_or_else(|| default_secret_label(name.as_str()));
        let credential = AppCredential::create(app.id().clone(), label, secret_hash);
        let agent = Agent::create(app.id().clone());
        let grant = Grant::new(
            Principal::App(app.id().clone()),
            RoleName::new(ORGANIZATION_AGENT_ROLE)?,
            Scope::Organization(organization_id),
        );
        self.app_repo()
            .provision_agent(&app, &credential, &agent, &grant)
            .await?;
        Ok(CreatedAgent { app, secret })
    }

    /// From the stream of the agent. No permission reaches the App of the agent (its grant is on an
    /// organization or a project), thus it is `Authenticated` and the target is the caller.
    #[usecase(write, access = Access::Authenticated)]
    async fn touch_agent(&self) -> DomainResult<DateTime<Utc>> {
        let app_id = app_only(self.ctx().caller())?;
        let at = clock::now();
        self.agent_repo().touch_last_seen(&app_id, at).await?;
        Ok(at)
    }

    /// From the stream of the agent, as `touch_agent`: the target is the caller.
    #[usecase(write, access = Access::Authenticated)]
    async fn record_agent_host(&self, host: AgentHost) -> DomainResult<AgentHost> {
        let app_id = app_only(self.ctx().caller())?;
        self.agent_repo().record_host(&app_id, &host).await?;
        Ok(host)
    }

    #[usecase(read, access = Access::Requires(Permission::ListAgents(organization_id.clone())))]
    async fn list_agents(&self, organization_id: OrganizationId) -> DomainResult<Vec<AgentView>> {
        let agents = self
            .agent_repo()
            .list_by_organization(&organization_id)
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

    #[usecase(read, access = Access::Requires(Permission::ReadApp(id.clone())))]
    async fn get_agent(&self, id: AppId) -> DomainResult<AgentView> {
        let id = &id;
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

    #[usecase(read, access = Access::Requires(Permission::ReadAppStats(id.clone())))]
    async fn get_agent_stats(&self, id: AppId) -> DomainResult<AgentStats> {
        let app = self.app_repo().find_by_id(&id).await?;
        self.agent_repo().agent_stats(app.id()).await
    }
}
