//! The `TestApp` helpers of `AgentService` for the job stats, the busy slots and the samples of
//! the machines of the agents.

use prost_types::Timestamp;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::agents::v1::agent_service_client::AgentServiceClient;
use scylla_proto::agents::v1::{
    AgentStats, GetAgentMetricsRequest, GetAgentMetricsResponse, GetAgentStatsRequest,
    GetSlotUsageRequest, GetSlotUsageResponse, ListAgentStatsRequest,
};
use tonic::Status;

use crate::{id, TestApp};

impl TestApp {
    /// Calls `GetAgentStats` as `actor` for the period from `from` to `to`, and returns the
    /// stats.
    pub async fn get_agent_stats(
        &self,
        actor: ActorId,
        app: &str,
        from: Option<Timestamp>,
        to: Option<Timestamp>,
    ) -> Result<AgentStats, Status> {
        let request = GetAgentStatsRequest {
            agent_id: Some(id(app)),
            from,
            to,
        };
        let request = self.authed(actor, request).await;
        let mut client = AgentServiceClient::new(self.channel.clone());
        let response = client.get_agent_stats(request).await?.into_inner();
        Ok(response.stats.expect("stats"))
    }

    /// Calls `ListAgentStats` as `actor` in a workspace, and returns the stats.
    pub async fn list_agent_stats(
        &self,
        actor: ActorId,
        workspace: WorkspaceId,
    ) -> Result<Vec<AgentStats>, Status> {
        let request = ListAgentStatsRequest {
            workspace_id: Some(id(workspace.to_string())),
        };
        let request = self.authed(actor, request).await;
        let mut client = AgentServiceClient::new(self.channel.clone());
        Ok(client.list_agent_stats(request).await?.into_inner().stats)
    }

    /// Calls `GetSlotUsage` as `actor`.
    pub async fn get_slot_usage(
        &self,
        actor: ActorId,
        request: GetSlotUsageRequest,
    ) -> Result<GetSlotUsageResponse, Status> {
        let request = self.authed(actor, request).await;
        let mut client = AgentServiceClient::new(self.channel.clone());
        Ok(client.get_slot_usage(request).await?.into_inner())
    }

    /// Calls `GetAgentMetrics` as `actor`.
    pub async fn get_agent_metrics(
        &self,
        actor: ActorId,
        request: GetAgentMetricsRequest,
    ) -> Result<GetAgentMetricsResponse, Status> {
        let request = self.authed(actor, request).await;
        let mut client = AgentServiceClient::new(self.channel.clone());
        Ok(client.get_agent_metrics(request).await?.into_inner())
    }
}
