//! The `TestApp` helpers of the API tokens of the Apps, and of their exchange.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::apps::v1::app_service_client::AppServiceClient;
use scylla_proto::apps::v1::{
    CreateAppTokenRequest, CreateAppTokenResponse, ListAppTokensRequest, RevokeAppTokenRequest,
};
use scylla_proto::auth::v1::token_service_client::TokenServiceClient;
use scylla_proto::auth::v1::{ApiToken, ExchangeApiTokenRequest};
use tonic::Status;

use crate::{id, TestApp};

impl TestApp {
    /// Calls `CreateAppToken` as `actor` with a name and an expiry (a `TokenExpiry` value).
    pub async fn create_app_token(
        &self,
        actor: ActorId,
        app: &str,
        name: &str,
        expiry: i32,
    ) -> Result<CreateAppTokenResponse, Status> {
        let request = CreateAppTokenRequest {
            app_id: Some(id(app)),
            name: name.to_owned(),
            expiry,
        };
        let request = self.authed(actor, request).await;
        let mut client = AppServiceClient::new(self.channel.clone());
        Ok(client.create_app_token(request).await?.into_inner())
    }

    /// Calls `ListAppTokens` as `actor`, and returns the tokens.
    pub async fn list_app_tokens(
        &self,
        actor: ActorId,
        app: &str,
    ) -> Result<Vec<ApiToken>, Status> {
        let request = ListAppTokensRequest {
            app_id: Some(id(app)),
        };
        let request = self.authed(actor, request).await;
        let mut client = AppServiceClient::new(self.channel.clone());
        Ok(client.list_app_tokens(request).await?.into_inner().tokens)
    }

    /// Calls `RevokeAppToken` as `actor`.
    pub async fn revoke_app_token(
        &self,
        actor: ActorId,
        app: &str,
        token: &str,
    ) -> Result<(), Status> {
        let request = RevokeAppTokenRequest {
            app_id: Some(id(app)),
            token_id: Some(id(token)),
        };
        let request = self.authed(actor, request).await;
        let mut client = AppServiceClient::new(self.channel.clone());
        client.revoke_app_token(request).await?;
        Ok(())
    }

    /// Calls `ExchangeApiToken` with a secret, and returns the short token.
    pub async fn exchange_api_token(&self, secret: &str) -> Result<String, Status> {
        let request = ExchangeApiTokenRequest {
            api_token: secret.to_owned(),
        };
        let mut client = TokenServiceClient::new(self.channel.clone());
        let response = client.exchange_api_token(request).await?.into_inner();
        Ok(response.token.expect("token").value)
    }
}
