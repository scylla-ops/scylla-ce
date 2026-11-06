//! The gRPC handler of `CapabilityService`.

use scylla_proto::capabilities::v1 as pb;
use scylla_proto::capabilities::v1::capability_service_server::{
    CapabilityService, CapabilityServiceServer,
};
use tonic::{Request, Response, Status};

/// Makes the service.
pub(crate) fn server(
    edition: &'static str,
    capabilities: &[&'static str],
    agent_image: String,
) -> CapabilityServiceServer<CapabilitiesGrpc> {
    CapabilityServiceServer::new(CapabilitiesGrpc {
        edition,
        capabilities: capabilities.iter().map(|name| (*name).to_owned()).collect(),
        agent_image,
    })
}

/// The handler. It holds the answer: the edition does not change after the start.
pub(crate) struct CapabilitiesGrpc {
    /// The name of the edition.
    edition: &'static str,
    /// The optional features of the edition.
    capabilities: Vec<String>,
    /// The image of the agent, with its tag.
    agent_image: String,
}

#[tonic::async_trait]
impl CapabilityService for CapabilitiesGrpc {
    async fn get_capabilities(
        &self,
        _request: Request<pb::GetCapabilitiesRequest>,
    ) -> Result<Response<pb::GetCapabilitiesResponse>, Status> {
        Ok(Response::new(pb::GetCapabilitiesResponse {
            edition: self.edition.to_owned(),
            capabilities: self.capabilities.clone(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            agent_image: self.agent_image.clone(),
        }))
    }
}
