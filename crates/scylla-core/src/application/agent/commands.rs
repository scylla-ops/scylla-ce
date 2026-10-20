//! The agent's writes. One block per command, in the order it runs: the struct, its access,
//! its payload types, what `Prepare` builds, what `Persist` writes. `TouchAgent` and
//! `RecordAgentHost` come from the agent's own stream: no permission reaches the agent's own
//! App, because its grant is on an organization or a project, so they are `Authenticated` and
//! the target is the caller.

use super::AgentUseCases;
use crate::application::actions::app_only;
use crate::application::app::mint_app_secret;
use crate::domain::agent::{Agent, AgentHost};
use crate::domain::app::{
    App, AppCredential, AppName, AppSecret, AppSecretLabel, default_secret_label,
};
use crate::domain::clock;
use crate::domain::errors::DomainResult;
use crate::domain::ids::{AppId, OrganizationId};
use crate::domain::permission::Permission;
use crate::domain::role::RoleName;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scylla_auth::authz::{Grant, ORGANIZATION_AGENT_ROLE, Principal, Scope};
use scylla_extension::{
    Access, Authorized, Command, Committed, Describe, Draft, Persist, Prepare, Prepared, Run,
};

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

#[async_trait]
impl Run<Prepare<CreateAgent>> for AgentUseCases {
    async fn run(&self, input: Authorized<CreateAgent>) -> DomainResult<Prepared<CreateAgent>> {
        let cmd = input.command();
        let app = App::create(cmd.organization_id.clone(), cmd.name.clone())?;
        let secret = mint_app_secret();
        let secret_hash = self.hash_service.hash_secret(&secret).await?;
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
        Ok(input.prepared(Draft::new(NewAgent {
            app,
            credential,
            agent,
            grant,
            secret,
        })))
    }
}

#[async_trait]
impl Run<Persist<CreateAgent>> for AgentUseCases {
    async fn run(&self, input: Prepared<CreateAgent>) -> DomainResult<Committed<CreateAgent>> {
        input
            .commit(async |draft| {
                let NewAgent {
                    app,
                    credential,
                    agent,
                    grant,
                    secret,
                } = draft.into_inner();
                self.app_repo
                    .provision_agent(&app, &credential, &agent, &grant)
                    .await?;
                Ok(CreatedAgent { app, secret })
            })
            .await
    }
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

#[async_trait]
impl Run<Prepare<TouchAgent>> for AgentUseCases {
    async fn run(&self, input: Authorized<TouchAgent>) -> DomainResult<Prepared<TouchAgent>> {
        let app_id = app_only(input.caller())?;
        Ok(input.prepared(Draft::new((app_id, clock::now()))))
    }
}

#[async_trait]
impl Run<Persist<TouchAgent>> for AgentUseCases {
    async fn run(&self, input: Prepared<TouchAgent>) -> DomainResult<Committed<TouchAgent>> {
        input
            .commit(async |draft| {
                let (app_id, at) = draft.into_inner();
                self.agent_repo.touch_last_seen(&app_id, at).await?;
                Ok(at)
            })
            .await
    }
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

#[async_trait]
impl Run<Prepare<RecordAgentHost>> for AgentUseCases {
    async fn run(
        &self,
        input: Authorized<RecordAgentHost>,
    ) -> DomainResult<Prepared<RecordAgentHost>> {
        let app_id = app_only(input.caller())?;
        let host = input.command().host.clone();
        Ok(input.prepared(Draft::new((app_id, host))))
    }
}

#[async_trait]
impl Run<Persist<RecordAgentHost>> for AgentUseCases {
    async fn run(
        &self,
        input: Prepared<RecordAgentHost>,
    ) -> DomainResult<Committed<RecordAgentHost>> {
        input
            .commit(async |draft| {
                let (app_id, host) = draft.into_inner();
                self.agent_repo.record_host(&app_id, &host).await?;
                Ok(host)
            })
            .await
    }
}
