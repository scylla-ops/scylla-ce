use crate::application::SecretUseCases;
use crate::grpc::adapter::run;
use crate::grpc::mappers::secret_to_proto;
use derive_more::Constructor;
use scylla_extension::Actions;
use scylla_proto::secret::v1::{
    CreateSecretRequest, CreateSecretResponse, DeleteSecretRequest, DeleteSecretResponse,
    ListSecretsRequest, ListSecretsResponse, secret_service_server::SecretService,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

#[derive(Constructor)]
pub struct SecretHandler {
    actions: Arc<Actions>,
    secrets: Arc<SecretUseCases>,
}

#[async_trait::async_trait]
impl SecretService for SecretHandler {
    async fn create_secret(
        &self,
        request: Request<CreateSecretRequest>,
    ) -> Result<Response<CreateSecretResponse>, Status> {
        let secret = run(&self.actions, &*self.secrets, request).await?;
        Ok(Response::new(CreateSecretResponse {
            secret: Some(secret_to_proto(&secret)),
        }))
    }

    async fn list_secrets(
        &self,
        request: Request<ListSecretsRequest>,
    ) -> Result<Response<ListSecretsResponse>, Status> {
        let secrets = run(&self.actions, &*self.secrets, request).await?;
        Ok(Response::new(ListSecretsResponse {
            secrets: secrets.iter().map(secret_to_proto).collect(),
        }))
    }

    async fn delete_secret(
        &self,
        request: Request<DeleteSecretRequest>,
    ) -> Result<Response<DeleteSecretResponse>, Status> {
        run(&self.actions, &*self.secrets, request).await?;
        Ok(Response::new(DeleteSecretResponse {}))
    }
}
