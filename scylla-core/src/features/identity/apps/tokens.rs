//! The handlers of `AppService` for the API tokens of an App.

use scylla_domain::identity::api_tokens::{ApiTokenId, TokenExpiry, TokenName};
use scylla_domain::identity::apps::{App, AppId};
use scylla_proto::apps::v1 as pb;
use scylla_proto::auth::v1::TokenExpiry as ProtoExpiry;
use tonic::Request;

use super::grpc::AppsGrpc;
use super::token_service;
use crate::api::parse::{self, ToProto};
use crate::Error;

impl AppsGrpc {
    /// Handles `CreateAppToken`: a new token, with its secret one time.
    pub(super) async fn create_token(
        &self,
        request: Request<pb::CreateAppTokenRequest>,
    ) -> Result<pb::CreateAppTokenResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let id: AppId = parse::required(request.app_id, "app_id")?;
        let name: TokenName = parse::field(request.name, "name")?;
        let expiry = expiry(request.expiry)?;
        self.authorize(&caller, App::CREATE_APP_TOKEN, id).await?;
        let created = token_service::create(&self.pool, id, &name, expiry, caller.actor.id);
        let (token, secret) = created.await?;
        Ok(pb::CreateAppTokenResponse {
            token: Some(token.to_proto()),
            secret,
        })
    }

    /// Handles `ListAppTokens`: the tokens of an App, the newest first.
    pub(super) async fn list_tokens(
        &self,
        request: Request<pb::ListAppTokensRequest>,
    ) -> Result<pb::ListAppTokensResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let id: AppId = parse::required(request.into_inner().app_id, "app_id")?;
        self.authorize(&caller, App::LIST_APP_TOKENS, id).await?;
        let tokens = token_service::list(&self.pool, id).await?;
        Ok(pb::ListAppTokensResponse {
            tokens: tokens.into_iter().map(ToProto::to_proto).collect(),
        })
    }

    /// Handles `RevokeAppToken`: the short tokens of the token stop at once.
    pub(super) async fn revoke_token(
        &self,
        request: Request<pb::RevokeAppTokenRequest>,
    ) -> Result<pb::RevokeAppTokenResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let id: AppId = parse::required(request.app_id, "app_id")?;
        let token: ApiTokenId = parse::required(request.token_id, "token_id")?;
        self.authorize(&caller, App::REVOKE_APP_TOKEN, id).await?;
        token_service::revoke(&self.pool, id, token).await?;
        Ok(pb::RevokeAppTokenResponse {})
    }
}

/// Reads the life of a new token. No value gives 90 days.
fn expiry(value: i32) -> Result<TokenExpiry, Error> {
    match ProtoExpiry::try_from(value) {
        Ok(ProtoExpiry::Unspecified | ProtoExpiry::Days90) => Ok(TokenExpiry::Days90),
        Ok(ProtoExpiry::Days7) => Ok(TokenExpiry::Days7),
        Ok(ProtoExpiry::Days30) => Ok(TokenExpiry::Days30),
        Ok(ProtoExpiry::Days365) => Ok(TokenExpiry::Days365),
        Err(_) => Err(Error::invalid("expiry", "must be a known value")),
    }
}
