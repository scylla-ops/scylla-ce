//! The `TestApp` helpers of the API tokens and of the CI identities of the Apps, and of their
//! exchanges.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::apps::v1::app_service_client::AppServiceClient;
use scylla_proto::apps::v1::{
    AddCiIdentityRequest, CiIdentity, CreateAppTokenRequest, CreateAppTokenResponse,
    ListAppTokensRequest, ListCiIdentitiesRequest, RemoveCiIdentityRequest, RevokeAppTokenRequest,
};
use scylla_proto::auth::v1::token_service_client::TokenServiceClient;
use scylla_proto::auth::v1::{
    ApiToken, ExchangeApiTokenRequest, ExchangeCiTokenRequest, TokenLimits,
};
use tonic::Status;

use crate::{id, TestApp};

impl TestApp {
    /// Calls `CreateAppToken` as `actor` with a name and an expiry (a `TokenExpiry` value), and the
    /// default limits.
    pub async fn create_app_token(
        &self,
        actor: ActorId,
        app: &str,
        name: &str,
        expiry: i32,
    ) -> Result<CreateAppTokenResponse, Status> {
        self.create_limited_app_token(actor, app, name, expiry, None)
            .await
    }

    /// Calls `CreateAppToken` as `actor` with a name, an expiry and limits.
    pub async fn create_limited_app_token(
        &self,
        actor: ActorId,
        app: &str,
        name: &str,
        expiry: i32,
        limits: Option<TokenLimits>,
    ) -> Result<CreateAppTokenResponse, Status> {
        let request = CreateAppTokenRequest {
            app_id: Some(id(app)),
            name: name.to_owned(),
            expiry,
            limits,
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

impl TestApp {
    /// Calls `AddCiIdentity` as `actor` with a request.
    pub async fn add_ci_identity(
        &self,
        actor: ActorId,
        request: AddCiIdentityRequest,
    ) -> Result<CiIdentity, Status> {
        let request = self.authed(actor, request).await;
        let mut client = AppServiceClient::new(self.channel.clone());
        let response = client.add_ci_identity(request).await?.into_inner();
        Ok(response.identity.expect("identity"))
    }

    /// Calls `ListCiIdentities` as `actor`.
    pub async fn list_ci_identities(
        &self,
        actor: ActorId,
        app: &str,
    ) -> Result<Vec<CiIdentity>, Status> {
        let request = ListCiIdentitiesRequest {
            app_id: Some(id(app)),
        };
        let request = self.authed(actor, request).await;
        let mut client = AppServiceClient::new(self.channel.clone());
        Ok(client
            .list_ci_identities(request)
            .await?
            .into_inner()
            .identities)
    }

    /// Calls `RemoveCiIdentity` as `actor`.
    pub async fn remove_ci_identity(
        &self,
        actor: ActorId,
        app: &str,
        identity: &str,
    ) -> Result<(), Status> {
        let request = RemoveCiIdentityRequest {
            app_id: Some(id(app)),
            identity_id: Some(id(identity)),
        };
        let request = self.authed(actor, request).await;
        let mut client = AppServiceClient::new(self.channel.clone());
        client.remove_ci_identity(request).await?;
        Ok(())
    }

    /// Calls `ExchangeCiToken` with the ID token of a CI job, and returns the short token.
    pub async fn exchange_ci_token(&self, app: &str, id_token: &str) -> Result<String, Status> {
        let request = ExchangeCiTokenRequest {
            app_id: Some(id(app)),
            id_token: id_token.to_owned(),
        };
        let mut client = TokenServiceClient::new(self.channel.clone());
        let response = client.exchange_ci_token(request).await?.into_inner();
        Ok(response.token.expect("token").value)
    }
}
