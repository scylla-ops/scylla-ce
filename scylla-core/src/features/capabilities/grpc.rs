use std::sync::Arc;

use scylla_extension::CapabilityProvider;
use scylla_proto::capabilities::v1 as pb;
use scylla_proto::capabilities::v1::capability_service_server::{
    CapabilityService, CapabilityServiceServer,
};
use tonic::{Request, Response, Status};

pub fn server(provider: Arc<dyn CapabilityProvider>) -> CapabilityServiceServer<CapabilitiesGrpc> {
    CapabilityServiceServer::new(CapabilitiesGrpc { provider })
}

pub struct CapabilitiesGrpc {
    provider: Arc<dyn CapabilityProvider>,
}

#[tonic::async_trait]
impl CapabilityService for CapabilitiesGrpc {
    async fn get_capabilities(
        &self,
        _request: Request<pb::GetCapabilitiesRequest>,
    ) -> Result<Response<pb::GetCapabilitiesResponse>, Status> {
        Ok(Response::new(pb::GetCapabilitiesResponse {
            edition: self.provider.edition().to_owned(),
            capabilities: self
                .provider
                .capabilities()
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }))
    }
}
