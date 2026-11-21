//! The `TestApp` helpers of the platform RPCs: the capabilities, the legal settings, the consent,
//! the health check and the gRPC reflection.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::capabilities::v1::capability_service_client::CapabilityServiceClient;
use scylla_proto::capabilities::v1::{GetCapabilitiesRequest, GetCapabilitiesResponse};
use scylla_proto::platform::v1::consent_service_client::ConsentServiceClient;
use scylla_proto::platform::v1::legal_service_client::LegalServiceClient;
use scylla_proto::platform::v1::{
    GetLegalInfoRequest, LegalInfo, RecordConsentRequest, RecordConsentResponse,
    UpdateLegalSettingsRequest,
};
use tonic::{Request, Status};
use tonic_health::pb::health_check_response::ServingStatus;
use tonic_health::pb::health_client::HealthClient;
use tonic_health::pb::HealthCheckRequest;
use tonic_reflection::pb::{v1, v1alpha};

use crate::TestApp;

impl TestApp {
    /// Calls `GetCapabilities`, with no token.
    pub async fn get_capabilities(&self) -> Result<GetCapabilitiesResponse, Status> {
        let mut client = CapabilityServiceClient::new(self.channel.clone());
        let response = client.get_capabilities(GetCapabilitiesRequest {}).await?;
        Ok(response.into_inner())
    }

    /// Calls `GetLegalInfo`, with no token.
    pub async fn get_legal_info(&self) -> Result<LegalInfo, Status> {
        let mut client = LegalServiceClient::new(self.channel.clone());
        let response = client.get_legal_info(GetLegalInfoRequest {}).await?;
        Ok(response.into_inner().info.expect("info"))
    }

    /// Calls `UpdateLegalSettings` as `actor`.
    pub async fn update_legal_settings(
        &self,
        actor: ActorId,
        request: UpdateLegalSettingsRequest,
    ) -> Result<LegalInfo, Status> {
        let request = self.authed(actor, request).await;
        let mut client = LegalServiceClient::new(self.channel.clone());
        let response = client.update_legal_settings(request).await?;
        Ok(response.into_inner().info.expect("info"))
    }

    /// Calls `RecordConsent` with no token, or as `actor`.
    pub async fn record_consent(
        &self,
        actor: Option<ActorId>,
        request: RecordConsentRequest,
    ) -> Result<RecordConsentResponse, Status> {
        let request = match actor {
            Some(actor) => self.authed(actor, request).await,
            None => Request::new(request),
        };
        let mut client = ConsentServiceClient::new(self.channel.clone());
        Ok(client.record_consent(request).await?.into_inner())
    }

    /// Calls the standard health check for the whole server.
    pub async fn health_check(&self) -> Result<ServingStatus, Status> {
        let mut client = HealthClient::new(self.channel.clone());
        let request = HealthCheckRequest {
            service: String::new(),
        };
        Ok(client.check(request).await?.into_inner().status())
    }

    /// Lists the services with the gRPC reflection v1, with no token.
    pub async fn reflection_services(&self) -> Result<Vec<String>, Status> {
        use v1::server_reflection_request::MessageRequest;
        use v1::server_reflection_response::MessageResponse;

        // Send one request on the stream, and read its answer.
        let mut client =
            v1::server_reflection_client::ServerReflectionClient::new(self.channel.clone());
        let request = v1::ServerReflectionRequest {
            host: String::new(),
            message_request: Some(MessageRequest::ListServices(String::new())),
        };
        let mut answers = client
            .server_reflection_info(tokio_stream::once(request))
            .await?
            .into_inner();
        let answer = answers
            .message()
            .await?
            .map(|answer| answer.message_response);

        // Keep the names of the services.
        match answer {
            Some(Some(MessageResponse::ListServicesResponse(list))) => Ok(list
                .service
                .into_iter()
                .map(|service| service.name)
                .collect()),
            other => Err(Status::internal(format!("not a list: {other:?}"))),
        }
    }

    /// Lists the services with the gRPC reflection v1alpha, with no token.
    pub async fn reflection_services_v1alpha(&self) -> Result<Vec<String>, Status> {
        use v1alpha::server_reflection_request::MessageRequest;
        use v1alpha::server_reflection_response::MessageResponse;

        // Send one request on the stream, and read its answer.
        let mut client =
            v1alpha::server_reflection_client::ServerReflectionClient::new(self.channel.clone());
        let request = v1alpha::ServerReflectionRequest {
            host: String::new(),
            message_request: Some(MessageRequest::ListServices(String::new())),
        };
        let mut answers = client
            .server_reflection_info(tokio_stream::once(request))
            .await?
            .into_inner();
        let answer = answers
            .message()
            .await?
            .map(|answer| answer.message_response);

        // Keep the names of the services.
        match answer {
            Some(Some(MessageResponse::ListServicesResponse(list))) => Ok(list
                .service
                .into_iter()
                .map(|service| service.name)
                .collect()),
            other => Err(Status::internal(format!("not a list: {other:?}"))),
        }
    }
}
