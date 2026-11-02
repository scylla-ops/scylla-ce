//! The gRPC handler of `AgentService`.

use std::sync::Arc;

use scylla_domain::agents::AgentHolder;
use scylla_domain::workspaces::WorkspaceId;
use scylla_extension::Gate;
use scylla_proto::agents::v1 as pb;
use scylla_proto::agents::v1::agent_service_server::{AgentService, AgentServiceServer};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::service;
use crate::parse::{self, ToProto};
use crate::Error;

/// Makes the service.
pub(super) fn server(pool: PgPool, gate: Arc<dyn Gate>) -> AgentServiceServer<AgentsGrpc> {
    AgentServiceServer::new(AgentsGrpc { pool, gate })
}

/// The handler.
pub(super) struct AgentsGrpc {
    /// The database pool.
    pool: PgPool,
    /// The access gate.
    gate: Arc<dyn Gate>,
}

#[tonic::async_trait]
impl AgentService for AgentsGrpc {
    async fn register_agent(
        &self,
        request: Request<pb::RegisterAgentRequest>,
    ) -> Result<Response<pb::RegisterAgentResponse>, Status> {
        Ok(Response::new(self.register(request).await?))
    }
}

impl AgentsGrpc {
    /// Handles `RegisterAgent`: an admin of a workspace registers an agent in it.
    async fn register(
        &self,
        request: Request<pb::RegisterAgentRequest>,
    ) -> Result<pb::RegisterAgentResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let workspace_id: WorkspaceId = parse::required(request.workspace_id, "workspace_id")?;

        // Check the route permission on the workspace.
        self.gate
            .authorize(&caller, AgentHolder::REGISTER_AGENT.on(workspace_id))
            .await?;

        // Register the agent in the workspace.
        let (agent_id, key_id) =
            service::register(&self.pool, workspace_id, &request.public_key).await?;
        Ok(pb::RegisterAgentResponse {
            agent_id: Some(agent_id.to_proto()),
            key_id: Some(key_id.to_proto()),
        })
    }
}
