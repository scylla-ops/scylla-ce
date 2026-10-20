use crate::application::{AgentUseCases, AppUseCases};
use crate::grpc::adapter::run;
use crate::grpc::mappers::{agent_stats_to_proto, agent_to_proto, agent_view_to_proto};
use derive_more::Constructor;
use scylla_extension::Actions;
use scylla_proto::agent::v1::{
    CreateAgentRequest, CreateAgentResponse, DeleteAgentRequest, DeleteAgentResponse,
    GetAgentRequest, GetAgentResponse, GetAgentStatsRequest, GetAgentStatsResponse,
    ListAgentsRequest, ListAgentsResponse, agent_admin_service_server::AgentAdminService,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

#[derive(Constructor)]
pub struct AgentAdminHandler {
    actions: Arc<Actions>,
    agents: Arc<AgentUseCases>,
    apps: Arc<AppUseCases>,
}

#[async_trait::async_trait]
impl AgentAdminService for AgentAdminHandler {
    async fn create_agent(
        &self,
        request: Request<CreateAgentRequest>,
    ) -> Result<Response<CreateAgentResponse>, Status> {
        let created = run(&self.actions, &*self.agents, request).await?;
        Ok(Response::new(CreateAgentResponse {
            agent: Some(agent_to_proto(&created.app)),
            secret: created.secret.as_str().to_string(),
        }))
    }

    async fn list_agents(
        &self,
        request: Request<ListAgentsRequest>,
    ) -> Result<Response<ListAgentsResponse>, Status> {
        let views = run(&self.actions, &*self.agents, request).await?;
        Ok(Response::new(ListAgentsResponse {
            agents: views.iter().map(agent_view_to_proto).collect(),
        }))
    }

    async fn get_agent(
        &self,
        request: Request<GetAgentRequest>,
    ) -> Result<Response<GetAgentResponse>, Status> {
        let view = run(&self.actions, &*self.agents, request).await?;
        Ok(Response::new(GetAgentResponse {
            agent: Some(agent_view_to_proto(&view)),
        }))
    }

    async fn get_agent_stats(
        &self,
        request: Request<GetAgentStatsRequest>,
    ) -> Result<Response<GetAgentStatsResponse>, Status> {
        let stats = run(&self.actions, &*self.agents, request).await?;
        Ok(Response::new(GetAgentStatsResponse {
            stats: Some(agent_stats_to_proto(&stats)),
        }))
    }

    async fn delete_agent(
        &self,
        request: Request<DeleteAgentRequest>,
    ) -> Result<Response<DeleteAgentResponse>, Status> {
        run(&self.actions, &*self.apps, request).await?;
        Ok(Response::new(DeleteAgentResponse {}))
    }
}
