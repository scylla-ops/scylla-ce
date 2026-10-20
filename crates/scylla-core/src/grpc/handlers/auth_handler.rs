use crate::application::{AuthUseCases, PasswordResetUseCases};
use crate::grpc::adapter::{run_public, run_public_with_client};
use crate::grpc::convert::wrap;
use crate::grpc::mappers::delivery_to_proto;
use scylla_extension::Actions;
use scylla_proto::auth::v1::{
    LoginRequest, LoginResponse, RequestPasswordResetRequest, RequestPasswordResetResponse,
    ResetPasswordRequest, ResetPasswordResponse, RevokeTokenRequest, RevokeTokenResponse,
    ValidateTokenRequest, ValidateTokenResponse, auth_service_server::AuthService,
    validate_token_response,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

pub struct AuthHandler {
    actions: Arc<Actions>,
    sessions: Arc<AuthUseCases>,
    resets: Arc<PasswordResetUseCases>,
    trust_forwarded_headers: bool,
}

impl AuthHandler {
    /// The handler does not trust the forwarded headers; see `trust_forwarded_headers`.
    #[must_use]
    pub fn new(
        actions: Arc<Actions>,
        sessions: Arc<AuthUseCases>,
        resets: Arc<PasswordResetUseCases>,
    ) -> Self {
        Self {
            actions,
            sessions,
            resets,
            trust_forwarded_headers: false,
        }
    }

    /// `[server].trust_forwarded_headers`: `Login` then reads the IP address of the client from
    /// `x-forwarded-for` or `x-real-ip` (`session_client`).
    #[must_use]
    pub fn trust_forwarded_headers(mut self, trust: bool) -> Self {
        self.trust_forwarded_headers = trust;
        self
    }
}

#[async_trait::async_trait]
impl AuthService for AuthHandler {
    async fn login(
        &self,
        request: Request<LoginRequest>,
    ) -> Result<Response<LoginResponse>, Status> {
        let session = run_public_with_client(
            &self.actions,
            &*self.sessions,
            request,
            self.trust_forwarded_headers,
        )
        .await?;
        Ok(Response::new(LoginResponse {
            token: session.token().to_string(),
            user_id: wrap(session.user_id().to_string()),
        }))
    }

    async fn validate_token(
        &self,
        request: Request<ValidateTokenRequest>,
    ) -> Result<Response<ValidateTokenResponse>, Status> {
        let result = if run_public(&self.actions, &*self.sessions, request).await? {
            validate_token_response::Result::Valid(validate_token_response::Valid {})
        } else {
            validate_token_response::Result::Invalid(validate_token_response::Invalid {})
        };
        Ok(Response::new(ValidateTokenResponse {
            result: Some(result),
        }))
    }

    async fn revoke_token(
        &self,
        request: Request<RevokeTokenRequest>,
    ) -> Result<Response<RevokeTokenResponse>, Status> {
        run_public(&self.actions, &*self.sessions, request).await?;
        Ok(Response::new(RevokeTokenResponse {}))
    }

    async fn request_password_reset(
        &self,
        request: Request<RequestPasswordResetRequest>,
    ) -> Result<Response<RequestPasswordResetResponse>, Status> {
        let delivery = run_public(&self.actions, &*self.resets, request).await?;
        Ok(Response::new(RequestPasswordResetResponse {
            delivery: delivery_to_proto(delivery),
        }))
    }

    async fn reset_password(
        &self,
        request: Request<ResetPasswordRequest>,
    ) -> Result<Response<ResetPasswordResponse>, Status> {
        run_public(&self.actions, &*self.resets, request).await?;
        Ok(Response::new(ResetPasswordResponse {}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::user::reset::ResetLinks;
    use crate::domain::ids::UserId;
    use crate::infrastructure::LogPasswordResetSender;
    use crate::test_support::authz::{DenyingPermissionService, actions};
    use crate::test_support::stubs::{
        OneUser, StubAccounts, StubHash, StubSessions, StubUsers, plain_hash,
    };
    use crate::test_support::users::UserBuilder;
    use scylla_proto::auth::v1::auth_service_client::AuthServiceClient;
    use scylla_proto::auth::v1::auth_service_server::AuthServiceServer;
    use tonic::service::Routes;
    use tonic::transport::server::TcpIncoming;
    use tonic::transport::{Endpoint, Server};

    const PASSWORD: &str = "SecurePass123!";

    #[tokio::test]
    async fn a_login_through_the_transport_server_records_the_peer_address_and_the_user_agent() {
        let kevin = UserBuilder::new("kevin")
            .id(UserId::new("kevin"))
            .password_hash(plain_hash(PASSWORD).as_str())
            .build();
        let sessions = Arc::new(StubSessions::default());
        let users = Arc::new(StubUsers::default());
        let handler = AuthHandler::new(
            Arc::new(actions(Arc::new(DenyingPermissionService::new()))),
            Arc::new(AuthUseCases::new(
                Arc::new(OneUser(kevin)),
                sessions.clone(),
                Arc::new(StubHash::plain()),
            )),
            Arc::new(PasswordResetUseCases::new(
                users.clone(),
                Arc::new(StubAccounts::new(users)),
                Arc::new(StubHash::plain()),
                Arc::new(LogPasswordResetSender),
                ResetLinks::new(None),
            )),
        );
        let incoming = TcpIncoming::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let address = incoming.local_addr().unwrap();
        let app = Routes::new(AuthServiceServer::new(handler)).into_axum_router();
        tokio::spawn(
            Server::builder()
                .add_routes(Routes::from(app))
                .serve_with_incoming(incoming),
        );
        let channel = Endpoint::from_shared(format!("http://{address}"))
            .unwrap()
            .user_agent("scylla-test/1.0")
            .unwrap()
            .connect()
            .await
            .unwrap();

        AuthServiceClient::new(channel)
            .login(LoginRequest {
                identifier: "kevin".into(),
                password: PASSWORD.into(),
            })
            .await
            .unwrap();

        let client = sessions.rows()[0].client().clone();
        assert_eq!(client.ip_address(), Some("127.0.0.1".parse().unwrap()));
        assert!(
            client
                .user_agent()
                .is_some_and(|agent| agent.starts_with("scylla-test/1.0")),
            "{client:?}"
        );
    }
}
