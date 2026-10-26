use super::{authed, parse};
use crate::{Secrets, SecretsRpc};
use scylla_core::grpc::domain_error_to_status;
use scylla_core::grpc::mappers::secret_to_proto;
use scylla_proto::secret::v1 as pb;
use strata_tonic::Grpc;

pub use super::service::secret::secret_service_client::SecretServiceClient;
pub use super::service::secret::secret_service_server::{SecretService, SecretServiceServer};

strata_tonic::service! {
    impl<A: Secrets> SecretService for Grpc<A> {
        rpc create_secret(pb::CreateSecretRequest) -> pb::CreateSecretResponse = |call, req| {
            authed(&call).await?;
            let secret = SecretsRpc::create_secret(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::CreateSecretResponse { secret: Some(secret_to_proto(&secret)) })
        }
        rpc list_secrets(pb::ListSecretsRequest) -> pb::ListSecretsResponse = |call, req| {
            authed(&call).await?;
            let secrets = SecretsRpc::list_secrets(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::ListSecretsResponse { secrets: secrets.iter().map(secret_to_proto).collect() })
        }
        rpc delete_secret(pb::DeleteSecretRequest) -> pb::DeleteSecretResponse = |call, req| {
            authed(&call).await?;
            SecretsRpc::delete_secret(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::DeleteSecretResponse {})
        }
    }
}
