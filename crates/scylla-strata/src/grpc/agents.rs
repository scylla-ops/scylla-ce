use super::{authed, parse};
use crate::access::CallerOf;
use crate::{AccessRpc, Agents, AgentsRpc, AppsRpc, JobControl, JobControlRpc};
use futures_util::StreamExt;
use scylla_core::application::agent::AgentStream;
use scylla_core::application::guards::app_only;
use scylla_core::grpc::agent_stream::{READ_CHUNK, Work, down_stream, hello_to_domain, work};
use scylla_core::grpc::convert::dt;
use scylla_core::grpc::domain_error_to_status;
use scylla_core::grpc::mappers::{agent_stats_to_proto, agent_to_proto, agent_view_to_proto};
use scylla_core::infrastructure::InMemoryAgentRegistry;
use scylla_domain::domain::clock;
use scylla_domain::domain::ids::JobId;
use scylla_domain::domain::job::JobLog;
use scylla_proto::agent::v1 as pb;
use std::pin::{Pin, pin};
use std::sync::Arc;
use strata::Contextual;
use strata_tonic::Grpc;
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status, Streaming};
use tracing::{debug, warn};

pub use super::service::agent::agent_admin_service_client::AgentAdminServiceClient;
pub use super::service::agent::agent_admin_service_server::{
    AgentAdminService, AgentAdminServiceServer,
};
pub use super::service::agent::agent_service_client::AgentServiceClient;
pub use super::service::agent::agent_service_server::{AgentService, AgentServiceServer};

strata_tonic::service! {
    impl<A: Agents> AgentAdminService for Grpc<A> {
        rpc create_agent(pb::CreateAgentRequest) -> pb::CreateAgentResponse = |call, req| {
            authed(&call).await?;
            let (organization_id, name, secret_label) = parse(req)?;
            let created = AgentsRpc::create_agent(&call.scoped(), organization_id, name, secret_label).await.map_err(domain_error_to_status)?;
            Ok(pb::CreateAgentResponse {
                agent: Some(agent_to_proto(&created.app)),
                secret: created.secret.as_str().to_string(),
            })
        }
        rpc list_agents(pb::ListAgentsRequest) -> pb::ListAgentsResponse = |call, req| {
            authed(&call).await?;
            let organization_id = parse(req)?;
            let views = AgentsRpc::list_agents(&call.scoped(), organization_id).await.map_err(domain_error_to_status)?;
            Ok(pb::ListAgentsResponse { agents: views.iter().map(agent_view_to_proto).collect() })
        }
        rpc get_agent(pb::GetAgentRequest) -> pb::GetAgentResponse = |call, req| {
            authed(&call).await?;
            let id = parse(req)?;
            let view = AgentsRpc::get_agent(&call.scoped(), id).await.map_err(domain_error_to_status)?;
            Ok(pb::GetAgentResponse { agent: Some(agent_view_to_proto(&view)) })
        }
        rpc get_agent_stats(pb::GetAgentStatsRequest) -> pb::GetAgentStatsResponse = |call, req| {
            authed(&call).await?;
            let id = parse(req)?;
            let stats = AgentsRpc::get_agent_stats(&call.scoped(), id).await.map_err(domain_error_to_status)?;
            Ok(pb::GetAgentStatsResponse { stats: Some(agent_stats_to_proto(&stats)) })
        }
        rpc delete_agent(pb::DeleteAgentRequest) -> pb::DeleteAgentResponse = |call, req| {
            authed(&call).await?;
            let id = parse(req)?;
            AppsRpc::delete_app(&call.scoped(), id).await.map_err(domain_error_to_status)?;
            Ok(pb::DeleteAgentResponse {})
        }
    }
}

#[tonic::async_trait]
impl<A: Agents + JobControl> AgentService for Grpc<A>
where
    <A as Contextual>::Ctx: CallerOf,
{
    type OpenStream =
        Pin<Box<dyn futures_util::Stream<Item = Result<pb::AgentDown, Status>> + Send + 'static>>;

    async fn open(
        &self,
        request: Request<Streaming<pb::AgentUp>>,
    ) -> Result<Response<Self::OpenStream>, Status> {
        let ctx = self.stream_context(&request).await?;
        let agent = {
            let scoped = self.server().scoped(&ctx);
            AccessRpc::authenticated(&scoped)
                .await
                .map_err(domain_error_to_status)?;
            AgentsRpc::touch_agent(&scoped)
                .await
                .map_err(domain_error_to_status)?;
            app_only(ctx.caller()).map_err(domain_error_to_status)?
        };
        let registry = self.app().agent_links().registry().clone();
        let conn = registry.register(&agent);
        let reader = Reader {
            grpc: self.clone(),
            ctx,
            registry,
            stream: conn.stream.clone(),
            stop: conn.stop.clone(),
        };
        tokio::spawn(reader.read(request.into_inner()));
        Ok(Response::new(Box::pin(down_stream(conn))))
    }
}

struct Reader<A: Contextual> {
    grpc: Grpc<A>,
    ctx: A::Ctx,
    registry: Arc<InMemoryAgentRegistry>,
    stream: AgentStream,
    stop: CancellationToken,
}

impl<A: Agents + JobControl> Reader<A> {
    async fn read(self, inbound: Streaming<pb::AgentUp>) {
        let stopped = self.stop.clone().cancelled_owned();
        let mut chunks = pin!(inbound.take_until(stopped).ready_chunks(READ_CHUNK));
        'read: while let Some(chunk) = chunks.next().await {
            for item in work(chunk) {
                if self.stop.is_cancelled() {
                    break 'read;
                }
                match item {
                    Work::Logs(job_id, logs) => self.append(job_id, logs).await,
                    Work::Status(status) => self.record(status).await,
                    Work::Hello(hello) => self.hello(hello).await,
                    Work::Undecodable(status) => {
                        warn!(app_id = %self.stream.agent, error = %status, "an agent frame does not decode; closing the stream");
                        break 'read;
                    }
                }
            }
        }
        self.touch().await;
        self.registry.unregister(&self.stream);
        let scoped = self.grpc.server().scoped(&self.ctx);
        if let Err(e) = JobControlRpc::release_agent_jobs(&scoped, self.stream.id.clone()).await {
            warn!(app_id = %self.stream.agent, error = %e, "failed to release the jobs placed on the stream");
        }
    }

    async fn touch(&self) {
        let scoped = self.grpc.server().scoped(&self.ctx);
        if let Err(e) = AgentsRpc::touch_agent(&scoped).await {
            warn!(app_id = %self.stream.agent, error = %e, "failed to update agent last_seen");
        }
    }

    async fn record(&self, status: pb::JobStatus) {
        let job_id = JobId::new(status.job_id.clone().unwrap_or_default().value);
        if let Some(event) = scylla_proto::convert::status_to_job_event(&status) {
            let at = dt(status.timestamp).unwrap_or_else(clock::now);
            let scoped = self.grpc.server().scoped(&self.ctx);
            if let Err(e) =
                JobControlRpc::record_job_status(&scoped, job_id.clone(), event, at).await
            {
                debug!(app_id = %self.stream.agent, job_id = %job_id, error = %e, "refused a job status");
            }
        }
        self.touch().await;
    }

    async fn append(&self, job_id: JobId, logs: Vec<JobLog>) {
        let scoped = self.grpc.server().scoped(&self.ctx);
        if let Err(e) = JobControlRpc::append_job_logs(&scoped, job_id.clone(), logs).await {
            debug!(app_id = %self.stream.agent, job_id = %job_id, error = %e, "refused job log lines");
        }
    }

    async fn hello(&self, hello: pb::AgentHello) {
        let scoped = self.grpc.server().scoped(&self.ctx);
        if let Err(e) = AgentsRpc::record_agent_host(&scoped, hello_to_domain(&hello)).await {
            warn!(app_id = %self.stream.agent, error = %e, "failed to record agent host");
        }
        let running = hello
            .running_jobs
            .into_iter()
            .map(|id| JobId::new(id.value))
            .collect();
        if let Err(e) = JobControlRpc::reconcile_agent_jobs(&scoped, running).await {
            warn!(app_id = %self.stream.agent, error = %e, "failed to reconcile the jobs of the agent");
        }
        self.touch().await;
    }
}
