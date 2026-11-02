//! The `TestApp` helpers of the platform RPCs: the capabilities and the health check.

use scylla_proto::capabilities::v1::capability_service_client::CapabilityServiceClient;
use scylla_proto::capabilities::v1::{GetCapabilitiesRequest, GetCapabilitiesResponse};
use tonic::Status;
use tonic_health::pb::health_check_response::ServingStatus;
use tonic_health::pb::health_client::HealthClient;
use tonic_health::pb::HealthCheckRequest;

use crate::TestApp;

impl TestApp {
    /// Calls `GetCapabilities`, with no token.
    pub async fn get_capabilities(&self) -> Result<GetCapabilitiesResponse, Status> {
        let mut client = CapabilityServiceClient::new(self.channel.clone());
        let response = client.get_capabilities(GetCapabilitiesRequest {}).await?;
        Ok(response.into_inner())
    }

    /// Calls the standard health check for the whole server.
    pub async fn health_check(&self) -> Result<ServingStatus, Status> {
        let mut client = HealthClient::new(self.channel.clone());
        let request = HealthCheckRequest {
            service: String::new(),
        };
        Ok(client.check(request).await?.into_inner().status())
    }
}
