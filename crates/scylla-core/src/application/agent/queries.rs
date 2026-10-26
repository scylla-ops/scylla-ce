//! The agent's reads. Each query, its access and its output type. The strata module of the slice
//! runs it.

use crate::application::AgentStats;
use crate::domain::agent::AgentHost;
use crate::domain::app::App;
use crate::domain::ids::{AppId, OrganizationId};
use crate::domain::permission::Permission;
use chrono::{DateTime, Utc};
use scylla_extension::{Access, Describe, Query};

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
