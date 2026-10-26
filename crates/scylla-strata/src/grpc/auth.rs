use super::parse;
use crate::{Auth, AuthRpc};
use scylla_core::grpc::convert::wrap;
use scylla_core::grpc::domain_error_to_status;
use scylla_core::grpc::mappers::delivery_to_proto;
use scylla_proto::auth::v1 as pb;
use scylla_proto::auth::v1::validate_token_response;
use strata_tonic::Grpc;

pub use super::service::auth::auth_service_client::AuthServiceClient;
pub use super::service::auth::auth_service_server::{AuthService, AuthServiceServer};

strata_tonic::service! {
    impl<A: Auth> AuthService for Grpc<A> {
        rpc login(pb::LoginRequest) -> pb::LoginResponse = |call, req| {
            let (identifier, password) = parse(req)?;
            let session = AuthRpc::login(&call.scoped(), identifier, password).await.map_err(domain_error_to_status)?;
            Ok(pb::LoginResponse { token: session.token().to_string(), user_id: wrap(session.user_id().to_string()) })
        }
        rpc validate_token(pb::ValidateTokenRequest) -> pb::ValidateTokenResponse = |call, req| {
            let token = parse(req)?;
            let result = if AuthRpc::validate_token(&call.scoped(), token).await.map_err(domain_error_to_status)? {
                validate_token_response::Result::Valid(validate_token_response::Valid {})
            } else {
                validate_token_response::Result::Invalid(validate_token_response::Invalid {})
            };
            Ok(pb::ValidateTokenResponse { result: Some(result) })
        }
        rpc revoke_token(pb::RevokeTokenRequest) -> pb::RevokeTokenResponse = |call, req| {
            let token = parse(req)?;
            AuthRpc::revoke_token(&call.scoped(), token).await.map_err(domain_error_to_status)?;
            Ok(pb::RevokeTokenResponse {})
        }
        rpc request_password_reset(pb::RequestPasswordResetRequest) -> pb::RequestPasswordResetResponse = |call, req| {
            let email = parse(req)?;
            let delivery = AuthRpc::request_password_reset(&call.scoped(), email).await.map_err(domain_error_to_status)?;
            Ok(pb::RequestPasswordResetResponse { delivery: delivery_to_proto(delivery) })
        }
        rpc reset_password(pb::ResetPasswordRequest) -> pb::ResetPasswordResponse = |call, req| {
            let (token, new_password) = parse(req)?;
            AuthRpc::reset_password(&call.scoped(), token, new_password).await.map_err(domain_error_to_status)?;
            Ok(pb::ResetPasswordResponse {})
        }
    }
}
