use super::parse;
use crate::{Auth, AuthRpc};
use scylla_core::grpc::convert::{ParseWithClient, wrap};
use scylla_core::grpc::domain_error_to_status;
use scylla_core::grpc::mappers::delivery_to_proto;
use scylla_domain::domain::session::SessionClient;
use scylla_proto::auth::v1 as pb;
use scylla_proto::auth::v1::validate_token_response;
use strata_tonic::Grpc;

pub use super::service::auth::auth_service_client::AuthServiceClient;
pub use super::service::auth::auth_service_server::{AuthService, AuthServiceServer};

strata_tonic::service! {
    impl<A: Auth> AuthService for Grpc<A> {
        rpc login(pb::LoginRequest) -> pb::LoginResponse = |call, req| {
            let login = req.parse_with_client(SessionClient::default())?;
            let session = AuthRpc::login(&call.scoped(), login).await.map_err(domain_error_to_status)?;
            Ok(pb::LoginResponse { token: session.token().to_string(), user_id: wrap(session.user_id().to_string()) })
        }
        rpc validate_token(pb::ValidateTokenRequest) -> pb::ValidateTokenResponse = |call, req| {
            let result = if AuthRpc::validate_token(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)? {
                validate_token_response::Result::Valid(validate_token_response::Valid {})
            } else {
                validate_token_response::Result::Invalid(validate_token_response::Invalid {})
            };
            Ok(pb::ValidateTokenResponse { result: Some(result) })
        }
        rpc revoke_token(pb::RevokeTokenRequest) -> pb::RevokeTokenResponse = |call, req| {
            AuthRpc::revoke_token(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::RevokeTokenResponse {})
        }
        rpc request_password_reset(pb::RequestPasswordResetRequest) -> pb::RequestPasswordResetResponse = |call, req| {
            let delivery = AuthRpc::request_password_reset(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::RequestPasswordResetResponse { delivery: delivery_to_proto(delivery) })
        }
        rpc reset_password(pb::ResetPasswordRequest) -> pb::ResetPasswordResponse = |call, req| {
            AuthRpc::reset_password(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::ResetPasswordResponse {})
        }
    }
}
