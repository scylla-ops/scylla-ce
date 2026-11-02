//! The gRPC handler of `TokenService`. Its RPCs need no token: they give one.

use std::sync::Arc;

use scylla_extension::{AccessError, Gate, IssuedToken};
use scylla_proto::auth::v1 as pb;
use scylla_proto::auth::v1::token_service_server::{TokenService, TokenServiceServer};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use crate::features::agents;
use crate::features::users::{self, OidcVerifier};
use crate::{parse, Error};

/// Makes the service. Without a verifier, `ExchangeIdToken` is refused.
pub(crate) fn server(
    pool: PgPool,
    gate: Arc<dyn Gate>,
    oidc: Option<Arc<OidcVerifier>>,
) -> TokenServiceServer<TokensGrpc> {
    TokenServiceServer::new(TokensGrpc { pool, gate, oidc })
}

/// The handler.
pub(crate) struct TokensGrpc {
    /// The database pool.
    pool: PgPool,
    /// The access gate, which signs the tokens.
    gate: Arc<dyn Gate>,
    /// The check of the ID tokens, if an issuer is set.
    oidc: Option<Arc<OidcVerifier>>,
}

#[tonic::async_trait]
impl TokenService for TokensGrpc {
    async fn exchange_agent_assertion(
        &self,
        request: Request<pb::ExchangeAgentAssertionRequest>,
    ) -> Result<Response<pb::ExchangeAgentAssertionResponse>, Status> {
        let request = request.into_inner();
        let token = self.agent_token(&request).await?;
        Ok(Response::new(pb::ExchangeAgentAssertionResponse {
            token: Some(token),
        }))
    }

    async fn exchange_id_token(
        &self,
        request: Request<pb::ExchangeIdTokenRequest>,
    ) -> Result<Response<pb::ExchangeIdTokenResponse>, Status> {
        let token = self.user_token(&request.into_inner().id_token).await?;
        Ok(Response::new(pb::ExchangeIdTokenResponse {
            token: Some(token),
        }))
    }
}

impl TokensGrpc {
    /// Checks the assertion of an agent and signs a token for it.
    async fn agent_token(
        &self,
        request: &pb::ExchangeAgentAssertionRequest,
    ) -> Result<pb::IssuedToken, Error> {
        let agent =
            agents::verify_assertion(&self.pool, &request.assertion, &request.signature).await?;
        let token = self.gate.issue(&agent)?;
        tracing::info!(actor = %agent.id, kind = agent.kind.as_str(), "token issued");
        Ok(to_proto(token))
    }

    /// Checks the ID token of a person and signs a token for the user.
    async fn user_token(&self, id_token: &str) -> Result<pb::IssuedToken, Error> {
        let verifier = self.oidc.as_ref().ok_or_else(|| {
            AccessError::Unauthenticated("no identity provider is set".to_owned())
        })?;
        let user = users::sign_in(&self.pool, self.gate.as_ref(), verifier, id_token).await?;
        let token = self.gate.issue(&user)?;
        tracing::info!(actor = %user.id, kind = user.kind.as_str(), "token issued");
        Ok(to_proto(token))
    }
}

/// Makes the proto of a token.
fn to_proto(token: IssuedToken) -> pb::IssuedToken {
    pb::IssuedToken {
        value: token.value,
        expires_at: Some(parse::timestamp(token.expires_at)),
    }
}
