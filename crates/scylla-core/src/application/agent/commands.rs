//! The agent's writes. Each command, its access and its payload types. The strata module of the
//! slice runs it. `TouchAgent` and `RecordAgentHost` come from the agent's own stream: no
//! permission reaches the agent's own App, because its grant is on an organization or a project, so
//! they are `Authenticated` and the target is the caller.

use crate::domain::agent::{Agent, AgentHost};
use crate::domain::app::{App, AppCredential, AppName, AppSecret, AppSecretLabel};
use crate::domain::ids::{AppId, OrganizationId};
use crate::domain::permission::Permission;
use chrono::{DateTime, Utc};
use scylla_auth::authz::Grant;
use scylla_extension::{Access, Command, Describe, Draft};

#[derive(Debug)]
pub struct CreateAgent {
    pub organization_id: OrganizationId,
    pub name: AppName,
    pub secret_label: Option<AppSecretLabel>,
}

/// No `Debug`: `secret` is the plaintext the caller sees once.
pub struct NewAgent {
    pub app: App,
    pub credential: AppCredential,
    pub agent: Agent,
    pub grant: Grant,
    pub secret: AppSecret,
}

pub struct CreatedAgent {
    pub app: App,
    pub secret: AppSecret,
}

impl Describe for CreateAgent {
    fn access(&self) -> Access {
        Access::Requires(Permission::CreateAgent(self.organization_id.clone()))
    }
}

impl Command for CreateAgent {
    type Staged = Draft<NewAgent>;
    type Committed = CreatedAgent;
}

#[derive(Debug)]
pub struct TouchAgent;

impl Describe for TouchAgent {
    fn access(&self) -> Access {
        Access::Authenticated
    }
}

impl Command for TouchAgent {
    type Staged = Draft<(AppId, DateTime<Utc>)>;
    type Committed = DateTime<Utc>;
}

#[derive(Debug)]
pub struct RecordAgentHost {
    pub host: AgentHost,
}

impl Describe for RecordAgentHost {
    fn access(&self) -> Access {
        Access::Authenticated
    }
}

impl Command for RecordAgentHost {
    type Staged = Draft<(AppId, AgentHost)>;
    type Committed = AgentHost;
}
