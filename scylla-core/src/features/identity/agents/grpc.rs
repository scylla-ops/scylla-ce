//! The gRPC handler of `AgentService`: an agent opens its stream. The stats are in `stats.rs`.

use std::sync::Arc;

use scylla_domain::identity::agents::AgentCaller;
use scylla_domain::projects::jobs::RunScope;
use scylla_extension::Gate;
use scylla_proto::agents::v1 as pb;
use scylla_proto::agents::v1::agent_service_server::{AgentService, AgentServiceServer};
use sqlx::PgPool;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};
use tracing::Instrument;

use super::stream::Stream;
use crate::features::identity::apps;
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// The number of messages that wait for a slow agent.
const OUTBOX_SIZE: usize = 16;

/// Makes the service, with what the Git integration shares: the open streams of the agents, the
/// key of the secrets, and the clients of the providers for the checkouts.
pub(crate) fn server(vcs: &IntegrationContext) -> AgentServiceServer<AgentsGrpc> {
    AgentServiceServer::new(AgentsGrpc {
        pool: vcs.pool.clone(),
        gate: Arc::clone(&vcs.gate),
        vcs: vcs.clone(),
    })
}

/// The handler.
pub(crate) struct AgentsGrpc {
    /// The database pool.
    pub(super) pool: PgPool,
    /// The access gate.
    pub(super) gate: Arc<dyn Gate>,
    /// What the Git integration shares: the open streams of the server, the key of the secrets
    /// and the clients of the providers.
    vcs: IntegrationContext,
}

#[tonic::async_trait]
impl AgentService for AgentsGrpc {
    type OpenStreamStream = ReceiverStream<Result<pb::OpenStreamResponse, Status>>;

    async fn open_stream(
        &self,
        request: Request<Streaming<pb::OpenStreamRequest>>,
    ) -> Result<Response<Self::OpenStreamStream>, Status> {
        Ok(Response::new(self.open(request).await?))
    }

    async fn get_agent_stats(
        &self,
        request: Request<pb::GetAgentStatsRequest>,
    ) -> Result<Response<pb::GetAgentStatsResponse>, Status> {
        Ok(Response::new(self.get_stats(request).await?))
    }

    async fn list_agent_stats(
        &self,
        request: Request<pb::ListAgentStatsRequest>,
    ) -> Result<Response<pb::ListAgentStatsResponse>, Status> {
        Ok(Response::new(self.list_stats(request).await?))
    }
}

impl AgentsGrpc {
    /// Handles `OpenStream`: checks the caller, then runs the stream in a task.
    async fn open(
        &self,
        request: Request<Streaming<pb::OpenStreamRequest>>,
    ) -> Result<ReceiverStream<Result<pb::OpenStreamResponse, Status>>, Error> {
        // Authenticate the caller, and check the route permission on itself.
        let caller = self.gate.caller(&request).await?;
        self.gate
            .authorize(&caller, AgentCaller::OPEN_STREAM.on(caller.actor.id))
            .await?;

        // Refuse an actor that is not an agent.
        let app = apps::runner(&self.pool, &caller.actor).await?;

        // Run the stream in a task, in the span of the call.
        let (out, messages) = mpsc::channel(OUTBOX_SIZE);
        let stream = Stream {
            pool: self.pool.clone(),
            gate: Arc::clone(&self.gate),
            registry: Arc::clone(&self.vcs.registry),
            sealer: self.vcs.sealer.clone(),
            vcs: self.vcs.clone(),
            app,
            principal: caller,
            workspace: None,
            capacity: 1,
            containers: false,
            checkout: false,
            scope: RunScope::Nothing,
        };
        let inbound = request.into_inner();
        tokio::spawn(stream.run(inbound, out).in_current_span());
        Ok(ReceiverStream::new(messages))
    }
}
