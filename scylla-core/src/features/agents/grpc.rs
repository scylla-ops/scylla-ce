//! The gRPC handler of `AgentService`.

use std::sync::Arc;

use scylla_extension::{Check, Gate, Object};
use scylla_proto::agents::v1 as pb;
use scylla_proto::agents::v1::agent_service_server::{AgentService, AgentServiceServer};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::service;
use crate::parse::ToProto;
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
    /// Handles `RegisterAgent`: the caller registers an agent that it can act for.
    async fn register(
        &self,
        request: Request<pb::RegisterAgentRequest>,
    ) -> Result<pb::RegisterAgentResponse, Error> {
        let principal = self.gate.authenticate(request.metadata()).await?;
        let caller = Object::new("actor", principal.actor.id);
        let check = Check {
            operation: "agents.register",
            object: &caller,
            permission: "act_for",
            at_least_as_fresh: None,
        };
        self.gate.authorize(&principal, check).await?;
        let public_key = request.into_inner().public_key;
        let (agent_id, key_id) = service::register(
            &self.pool,
            self.gate.as_ref(),
            principal.actor.id,
            &public_key,
        )
        .await?;
        Ok(pb::RegisterAgentResponse {
            agent_id: Some(agent_id.to_proto()),
            key_id: Some(key_id.to_proto()),
        })
    }
}
