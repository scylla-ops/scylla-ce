//! The gRPC handler of `PersonalTokenService`: a user manages its own personal tokens.

use std::sync::Arc;

use scylla_domain::authz::Permission;
use scylla_domain::identity::actors::Actor;
use scylla_domain::identity::api_tokens::{ApiTokenId, TokenName, TokenOwner};
use scylla_domain::identity::users::KIND;
use scylla_domain::tenancy::workspaces::{VisibleWorkspaces, Workspace};
use scylla_extension::{AccessError, Gate, Principal};
use scylla_proto::accounts::v1 as pb;
use scylla_proto::accounts::v1::personal_token_service_server::{
    PersonalTokenService, PersonalTokenServiceServer,
};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::service;
use crate::api::parse::{self, ToProto};
use crate::features::identity::api_tokens;
use crate::features::tenancy::workspaces;
use crate::Error;

/// Makes the service.
pub(crate) fn server(
    pool: PgPool,
    gate: Arc<dyn Gate>,
) -> PersonalTokenServiceServer<PersonalTokensGrpc> {
    PersonalTokenServiceServer::new(PersonalTokensGrpc { pool, gate })
}

/// The handler.
pub(crate) struct PersonalTokensGrpc {
    /// The database pool.
    pool: PgPool,
    /// The access gate.
    gate: Arc<dyn Gate>,
}

#[tonic::async_trait]
impl PersonalTokenService for PersonalTokensGrpc {
    async fn create_personal_token(
        &self,
        request: Request<pb::CreatePersonalTokenRequest>,
    ) -> Result<Response<pb::CreatePersonalTokenResponse>, Status> {
        Ok(Response::new(self.create(request).await?))
    }

    async fn list_personal_tokens(
        &self,
        request: Request<pb::ListPersonalTokensRequest>,
    ) -> Result<Response<pb::ListPersonalTokensResponse>, Status> {
        Ok(Response::new(self.list(request).await?))
    }

    async fn revoke_personal_token(
        &self,
        request: Request<pb::RevokePersonalTokenRequest>,
    ) -> Result<Response<pb::RevokePersonalTokenResponse>, Status> {
        Ok(Response::new(self.revoke(request).await?))
    }
}

impl PersonalTokensGrpc {
    /// Handles `CreatePersonalToken`: a new token, with its secret one time.
    async fn create(
        &self,
        request: Request<pb::CreatePersonalTokenRequest>,
    ) -> Result<pb::CreatePersonalTokenResponse, Error> {
        let caller = self
            .user(&request, TokenOwner::CREATE_PERSONAL_TOKEN)
            .await?;
        api_tokens::check_maker(&caller)?;
        let request = request.into_inner();
        let name: TokenName = parse::field(request.name, "name")?;
        let expiry = api_tokens::expiry(request.expiry)?;
        let limits = api_tokens::limits(request.limits)?;
        let user = caller.actor.id;
        let (token, secret) = service::create(&self.pool, user, &name, expiry, &limits).await?;
        Ok(pb::CreatePersonalTokenResponse {
            token: Some(token.to_proto()),
            secret,
        })
    }

    /// Handles `ListPersonalTokens`: the tokens of the caller, and where they act.
    async fn list(
        &self,
        request: Request<pb::ListPersonalTokensRequest>,
    ) -> Result<pb::ListPersonalTokensResponse, Error> {
        let caller = self
            .user(&request, TokenOwner::LIST_PERSONAL_TOKENS)
            .await?;
        let (tokens, enabled) = service::list(&self.pool, caller.actor.id).await?;

        // Find the workspaces of the caller that refuse the personal tokens.
        let granted = self
            .gate
            .granted(&caller, Workspace::LIST_WORKSPACES)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "a lookup failed: the list shows only the owned workspaces");
                Vec::new()
            });
        let visible = VisibleWorkspaces {
            caller: caller.actor.id,
            granted,
        };
        let refused = workspaces::refusing_personal_tokens(&self.pool, visible).await?;
        Ok(pb::ListPersonalTokensResponse {
            tokens: tokens.into_iter().map(ToProto::to_proto).collect(),
            enabled,
            refused_workspace_ids: refused.into_iter().map(ToProto::to_proto).collect(),
        })
    }

    /// Handles `RevokePersonalToken`: the short tokens of the token stop at once.
    async fn revoke(
        &self,
        request: Request<pb::RevokePersonalTokenRequest>,
    ) -> Result<pb::RevokePersonalTokenResponse, Error> {
        let caller = self
            .user(&request, TokenOwner::REVOKE_PERSONAL_TOKEN)
            .await?;
        let token: ApiTokenId = parse::required(request.into_inner().token_id, "token_id")?;
        service::revoke(&self.pool, caller.actor.id, token).await?;
        Ok(pb::RevokePersonalTokenResponse {})
    }

    /// Authenticates a caller that is a user, and checks the route permission on itself.
    async fn user<T: Sync>(
        &self,
        request: &Request<T>,
        permission: Permission<Actor>,
    ) -> Result<Principal, Error> {
        let caller = self.gate.caller(request).await?;
        if caller.actor.kind != KIND {
            let reason = "only a user has personal tokens".to_owned();
            return Err(AccessError::Denied(reason).into());
        }
        self.gate
            .authorize(&caller, permission.on(caller.actor.id))
            .await?;
        Ok(caller)
    }
}
